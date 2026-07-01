use std::collections::HashMap;
use std::path::Path;

use axum::body::Body;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use http::header;
use image::ImageFormat;
use tokio::io::AsyncReadExt;
use tokio_util::io::ReaderStream;

use crate::config::types::ImageResizeConfig;
use crate::config::types::StaticFilesConfig;
use crate::error::AppError;
use crate::handlers::static_files::routing::StaticGetContext;
use crate::handlers::static_files::utils::{
    apply_cache_control, apply_content_type, mime_from_path,
};
use crate::server::AppState;
use crate::storage::Storage;

/// Handle GET requests - serve files with optional image resize or streaming.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if path traversal is detected.
/// Returns `AppError::NotFound` if the file is not found.
pub(crate) async fn handle_static_get(
    _state: State<AppState>,
    ctx: StaticGetContext<'_>,
) -> Result<Response, AppError> {
    let storage = &*ctx.storage;
    let resolved = if ctx.relative.is_empty() {
        ctx.root.to_path_buf()
    } else {
        let r = ctx.root.join(ctx.relative);
        if let (Ok(canonical), Ok(root_canonical)) = (
            storage.canonicalize(&r).await,
            storage.canonicalize(ctx.root).await,
        ) && !canonical.starts_with(&root_canonical)
        {
            return Err(AppError::Forbidden("Path traversal denied".to_string()));
        }
        r
    };

    let resolved_meta = storage.metadata(&resolved).await.ok();

    // If path points to a directory (not a file), try index file or directory listing
    if resolved_meta.as_ref().is_some_and(|m| !m.is_file) {
        let index_path = resolved.join(&ctx.config.index);
        let index_exists = storage.metadata(&index_path).await.is_ok_and(|m| m.is_file);
        if index_exists {
            return serve_file(storage, &index_path, ctx.config, None, ctx.headers).await;
        }
        if ctx.config.directory_listing {
            return crate::handlers::static_files::directory::generate_directory_listing(
                storage,
                &resolved,
                ctx.request_path,
            )
            .await;
        }
    }

    let file_exists = resolved_meta.as_ref().is_some_and(|m| m.is_file);
    if !file_exists {
        return Err(AppError::NotFound(format!(
            "File not found: {0}",
            ctx.request_path
        )));
    }

    serve_file(
        storage,
        &resolved,
        ctx.config,
        ctx.query.map(|q| q.0.clone()),
        ctx.headers,
    )
    .await
}

/// Serve a single file with optional image resize or streaming.
pub async fn serve_file(
    storage: &dyn Storage,
    path: &Path,
    config: &StaticFilesConfig,
    query_params: Option<HashMap<String, String>>,
    headers: &axum::http::HeaderMap,
) -> Result<Response, AppError> {
    let meta = storage
        .metadata(path)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let file_size = meta.size;
    let content_type = mime_from_path(path);

    // Check if image resize is requested.
    if let Some(image_config) = &config.image_resize
        && image_config.enabled
        && is_image_path(path)
        && let Some(resized) =
            handle_image_resize(storage, path, query_params.as_ref(), image_config).await?
    {
        return Ok(resized);
    }

    // Handle range requests.
    if config.range_requests
        && let Some(range_header) = headers.get(header::RANGE)
        && let Ok(range_str) = range_header.to_str()
    {
        // Determine streaming threshold.
        let streaming_threshold = config
            .streaming
            .as_ref()
            .and_then(|s| s.enabled.then_some(s.threshold))
            .unwrap_or(u64::MAX);

        if file_size > streaming_threshold {
            // Use streaming for large files.
            #[allow(clippy::expect_used)]
            // streaming_config is guaranteed: if streaming_threshold < u64::MAX then config.streaming was Some
            let streaming_config = config
                .streaming
                .as_ref()
                .expect("streaming enabled when threshold < u64::MAX");
            return handle_range_streaming(
                storage,
                path,
                range_str,
                file_size,
                content_type,
                streaming_config,
                config.cache_max_age,
            )
            .await;
        }

        // For small files, read full content and extract range.
        let content = storage
            .read(path)
            .await
            .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

        return handle_range_small_file(
            &content,
            file_size,
            range_str,
            content_type,
            config.cache_max_age,
        );
    }

    // Check if streaming is enabled.
    if let Some(streaming_config) = &config.streaming
        && streaming_config.enabled
        && file_size > streaming_config.threshold
    {
        return handle_streaming(
            storage,
            path,
            content_type,
            streaming_config,
            config.cache_max_age,
            file_size,
        )
        .await;
    }

    // Default: read the full file into memory for small files.
    let content = storage
        .read(path)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let mut response = (StatusCode::OK, content.to_vec()).into_response();
    apply_content_type(&mut response, content_type);
    apply_cache_control(
        &mut response,
        config.cache_max_age,
        Some(path),
        &config.cache_rules,
    );

    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&content.len().to_string()).unwrap_or(HeaderValue::from_static("0")),
    );

    Ok(response)
}

/// Handle a range request by streaming the requested byte range from storage.
///
/// Opens the file at the start offset using `seek_read` and streams only the
/// requested number of bytes, avoiding loading the full file into memory.
async fn handle_range_streaming(
    storage: &dyn Storage,
    path: &Path,
    range_str: &str,
    file_size: u64,
    content_type: &str,
    streaming_config: &crate::config::types::StreamingConfig,
    cache_max_age: u64,
) -> Result<Response, AppError> {
    // Parse Range: bytes=START-END
    let range_value = range_str
        .strip_prefix("bytes=")
        .ok_or_else(|| AppError::BadRequest("Invalid Range header".to_string()))?;

    let (start_str, end_str) = range_value
        .split_once('-')
        .ok_or_else(|| AppError::BadRequest("Invalid Range format".to_string()))?;

    // Parse start position
    let start: u64 = start_str
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid Range start".to_string()))?;

    // Parse end position (optional - if empty, use file_size - 1)
    let end: Option<u64> = if end_str.is_empty() {
        Some(file_size.saturating_sub(1))
    } else {
        end_str.parse().ok()
    };

    let end = end
        .unwrap_or(file_size.saturating_sub(1))
        .min(file_size.saturating_sub(1));

    // Validate range
    if start >= file_size || start > end {
        return Ok(build_range_not_satisfiable_response(
            file_size,
            cache_max_age,
        ));
    }

    let content_length = end.saturating_add(1).saturating_sub(start);

    // Open file at start offset and stream only the requested range.
    let reader = storage
        .seek_read(path, start)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let taken = reader.take(content_length);

    let reader_stream = ReaderStream::with_capacity(taken, streaming_config.buffer_size);
    let body = Body::from_stream(reader_stream);

    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::PARTIAL_CONTENT;

    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );

    response.headers_mut().insert(
        header::CONTENT_RANGE,
        HeaderValue::from_str(&format!("bytes {start}-{end}/{file_size}"))
            .unwrap_or(HeaderValue::from_static("bytes */0")),
    );

    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&content_length.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );

    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));

    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str(&format!("public, max-age={}", cache_max_age))
            .unwrap_or(HeaderValue::from_static("public, max-age=3600")),
    );

    Ok(response)
}

/// Handle full file streaming from storage without loading the entire file
/// into memory.
///
/// Opens the file at offset 0 using `seek_read` and streams the entire
/// file content.
async fn handle_streaming(
    storage: &dyn Storage,
    path: &Path,
    content_type: &str,
    streaming_config: &crate::config::types::StreamingConfig,
    cache_max_age: u64,
    file_size: u64,
) -> Result<Response, AppError> {
    let reader = storage
        .seek_read(path, 0)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let reader_stream = ReaderStream::with_capacity(reader, streaming_config.buffer_size);
    let body = Body::from_stream(reader_stream);

    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::OK;

    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );

    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&format!("{file_size}")).unwrap_or(HeaderValue::from_static("0")),
    );

    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str(&format!("public, max-age={}", cache_max_age))
            .unwrap_or(HeaderValue::from_static("public, max-age=3600")),
    );

    Ok(response)
}

/// Build response for range not satisfiable (416).
#[must_use]
fn build_range_not_satisfiable_response(file_size: u64, cache_max_age: u64) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;

    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );

    response.headers_mut().insert(
        header::CONTENT_RANGE,
        HeaderValue::from_str(&format!("bytes */{file_size}"))
            .unwrap_or(HeaderValue::from_static("bytes */0")),
    );

    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str(&format!("public, max-age={cache_max_age}"))
            .unwrap_or(HeaderValue::from_static("public, max-age=3600")),
    );

    response
}

/// Handle image resize if parameters are present.
pub(crate) async fn handle_image_resize(
    storage: &dyn Storage,
    path: &Path,
    query_params: Option<&HashMap<String, String>>,
    config: &ImageResizeConfig,
) -> Result<Option<Response>, AppError> {
    let empty_map = HashMap::new();
    let query = query_params.unwrap_or(&empty_map);

    let width = query.get("w").and_then(|w| w.parse().ok());
    let height = query.get("h").and_then(|h| h.parse().ok());
    let fit = query.get("fit").map(std::string::String::as_str);
    let output_format = query.get("format").map(std::string::String::as_str);

    if width.is_none() && height.is_none() && output_format.is_none() {
        return Ok(None);
    }

    // Read the original image.
    let image_data = storage
        .read(path)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let format = image::guess_format(&image_data)
        .map_err(|_| AppError::BadRequest("Invalid image format".to_string()))?;

    let img = image::load_from_memory(&image_data)
        .map_err(|_| AppError::BadRequest("Invalid image data".to_string()))?;

    // Determine target dimensions.
    let (target_width, target_height) = match (width, height, fit) {
        (Some(w), None, _) => (Some(w), None),
        (None, Some(h), _) => (None, Some(h)),
        (Some(w), Some(h), Some("cover")) => {
            // Fill while maintaining aspect ratio (cover mode)
            let ratio = img.width() as f64 / img.height() as f64;
            let h_ratio = h as f64 / w as f64;
            if ratio > h_ratio {
                let new_h = (w as f64 / ratio) as u32;
                (Some(w), Some(new_h))
            } else {
                let new_w = (h as f64 * ratio) as u32;
                (Some(new_w), Some(h))
            }
        }
        (Some(w), Some(h), _) => {
            let max_dim = u32::try_from(config.max_dimension).unwrap_or(u32::MAX);
            (Some(w.clamp(1, max_dim)), Some(h.clamp(1, max_dim)))
        }
        (None, None, None | Some(_)) => (None, None),
    };

    let resized = if let (Some(w), Some(h)) = (target_width, target_height) {
        img.resize(w, h, image::imageops::FilterType::Lanczos3)
    } else if let Some(w) = target_width {
        img.resize_to_fill(w, img.height(), image::imageops::FilterType::Lanczos3)
    } else if let Some(h) = target_height {
        img.resize_to_fill(img.width(), h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    // Determine output format.
    let output_format = output_format
        .and_then(image::ImageFormat::from_extension)
        .unwrap_or(format);

    // Encode the image.
    let mut output_bytes = Vec::new();
    resized
        .write_to(&mut std::io::Cursor::new(&mut output_bytes), output_format)
        .map_err(|_| AppError::Internal("Failed to encode image".to_string()))?;

    let content_type = match output_format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::WebP => "image/webp",
        _ => "application/octet-stream",
    };

    let mut response = (StatusCode::OK, output_bytes).into_response();
    apply_content_type(&mut response, content_type);
    Ok(Some(response))
}

/// Check if a file path likely refers to an image based on its extension.
#[must_use]
pub(crate) fn is_image_path(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext_lower = ext.to_lowercase();
        matches!(
            ext_lower.as_str(),
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg"
        )
    } else {
        false
    }
}

/// Handle range requests for small files (in-memory).
fn handle_range_small_file(
    content: &[u8],
    file_size: u64,
    range_str: &str,
    content_type: &str,
    cache_max_age: u64,
) -> Result<Response, AppError> {
    // Parse range header: "bytes=start-end" or "bytes=start-"
    let bytes_range = range_str
        .strip_prefix("bytes=")
        .ok_or_else(|| AppError::BadRequest("Invalid Range header format".to_string()))?;

    let (start, end) = if let Some((s, e)) = bytes_range.split_once('-') {
        let start: u64 = s
            .parse()
            .map_err(|_| AppError::BadRequest("Invalid Range header: invalid start".to_string()))?;
        let end: u64 = if e.is_empty() {
            file_size - 1
        } else {
            e.parse().map_err(|_| {
                AppError::BadRequest("Invalid Range header: invalid end".to_string())
            })?
        };
        (start, end)
    } else {
        return Err(AppError::BadRequest(
            "Invalid Range header format".to_string(),
        ));
    };

    if start >= file_size {
        return Err(AppError::RequestedRangeNotSatisfiable(format!(
            "Range bytes={}-{} is not satisfiable for file size {file_size}",
            start, end
        )));
    }

    let actual_end = end.min(file_size - 1);
    let range_size = actual_end - start + 1;

    let body = content[start as usize..(actual_end + 1) as usize].to_vec();

    let mut response = (StatusCode::PARTIAL_CONTENT, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&range_size.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );
    response.headers_mut().insert(
        header::CONTENT_RANGE,
        HeaderValue::from_str(&format!("bytes {}-{}/{}", start, actual_end, file_size))
            .unwrap_or(HeaderValue::from_static("bytes */0")),
    );

    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str(&format!("public, max-age={}", cache_max_age))
            .unwrap_or(HeaderValue::from_static("public, max-age=0")),
    );

    Ok(response)
}

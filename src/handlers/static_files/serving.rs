//! File serving with caching, `ETags`, and range requests.

use std::collections::HashMap;
use std::path::Path;

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use http::header;
use tokio::io::AsyncReadExt;
use tokio_util::io::ReaderStream;

use crate::config::types::{CacheRuleConfig, ImageResizeConfig, StaticFilesConfig, mime_from_path};
use crate::error::AppError;
use crate::handlers::common::helpers::is_image_extension;
use crate::handlers::common::resize::{
    ResizeParams, build_resize_response, parse_resize_params, resize_image,
};
use crate::handlers::common::utils::{
    apply_cache_control, apply_content_length, apply_content_range, apply_content_type,
};
use crate::handlers::static_files::routing::StaticGetContext;
use crate::storage::Storage;

/// Handle GET requests - serve files with optional image resize or streaming.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if path traversal is detected.
/// Returns `AppError::NotFound` if the file is not found.
pub(crate) async fn handle_static_get(ctx: StaticGetContext<'_>) -> Result<Response, AppError> {
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
///
/// # Errors
///
/// Returns an `AppError::NotFound` if the file is not found in storage.
#[allow(clippy::implicit_hasher)]
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
        && is_image_extension(path.extension().and_then(|e| e.to_str()).unwrap_or(""))
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

        if file_size > streaming_threshold
            && let Some(streaming_config) = config.streaming.as_ref()
        {
            return handle_range_streaming(
                storage,
                path,
                range_str,
                file_size,
                content_type,
                streaming_config,
                config,
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
            path,
            &config.cache_rules,
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
            &config.cache_rules,
        )
        .await;
    }

    // Default: read the full file into memory for small files.
    let content = storage
        .read(path)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let mut response = (StatusCode::OK, content.clone()).into_response();
    apply_content_type(&mut response, content_type);
    apply_cache_control(
        &mut response,
        config.cache_max_age,
        Some(path),
        &config.cache_rules,
    );
    apply_content_length(&mut response, &content.len().to_string());

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
    config: &StaticFilesConfig,
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
        .unwrap_or_else(|| file_size.saturating_sub(1))
        .min(file_size.saturating_sub(1));

    // Validate range
    if start >= file_size || start > end {
        return Err(AppError::RequestedRangeNotSatisfiable {
            message: format!(
                "Range bytes={start}-{end} is not satisfiable for file size {file_size}"
            ),
            content_range: Some(format!("bytes */{file_size}")),
        });
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

    apply_content_type(&mut response, content_type);
    apply_content_range(&mut response, &format!("bytes {start}-{end}/{file_size}"));
    apply_content_length(&mut response, &content_length.to_string());

    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));

    apply_cache_control(
        &mut response,
        config.cache_max_age,
        Some(path),
        &config.cache_rules,
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
    cache_rules: &[CacheRuleConfig],
) -> Result<Response, AppError> {
    let reader = storage
        .seek_read(path, 0)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let reader_stream = ReaderStream::with_capacity(reader, streaming_config.buffer_size);
    let body = Body::from_stream(reader_stream);

    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::OK;

    apply_content_length(&mut response, &format!("{file_size}"));
    apply_content_type(&mut response, content_type);
    apply_cache_control(&mut response, cache_max_age, Some(path), cache_rules);

    Ok(response)
}

/// Handle image resize if parameters are present.
pub(crate) async fn handle_image_resize(
    storage: &dyn Storage,
    path: &Path,
    query_params: Option<&std::collections::HashMap<String, String>>,
    config: &ImageResizeConfig,
) -> Result<Option<Response>, AppError> {
    let params = query_params.map_or(
        ResizeParams {
            width: None,
            height: None,
            fit: None,
            output_format: None,
        },
        |qp| parse_resize_params(qp),
    );

    if params.width.is_none() && params.height.is_none() && params.output_format.is_none() {
        return Ok(None);
    }

    // Read the original image.
    let image_data = storage
        .read(path)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let format = image::guess_format(&image_data)
        .map_err(|_| AppError::BadRequest("Invalid image format".to_string()))?;

    let (resized_bytes, content_type) = resize_image(
        &image_data,
        &params,
        format,
        u32::try_from(config.max_dimension).unwrap_or(u32::MAX),
    )?;

    let response = build_resize_response(resized_bytes, content_type, None);
    Ok(Some(response))
}

/// Handle range requests for small files (in-memory).
fn handle_range_small_file(
    content: &[u8],
    file_size: u64,
    range_str: &str,
    content_type: &str,
    cache_max_age: u64,
    path: &Path,
    cache_rules: &[CacheRuleConfig],
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
        return Err(AppError::RequestedRangeNotSatisfiable {
            message: format!(
                "Range bytes={start}-{end} is not satisfiable for file size {file_size}"
            ),
            content_range: Some(format!("bytes */{file_size}")),
        });
    }

    let actual_end = end.min(file_size - 1);
    let range_size = actual_end - start + 1;

    #[allow(clippy::cast_possible_truncation)]
    let body = content[start as usize..(actual_end + 1) as usize].to_vec();

    let mut response = (StatusCode::PARTIAL_CONTENT, body).into_response();

    apply_content_type(&mut response, content_type);
    apply_content_length(&mut response, &range_size.to_string());
    apply_content_range(
        &mut response,
        &format!("bytes {start}-{actual_end}/{file_size}"),
    );
    apply_cache_control(&mut response, cache_max_age, Some(path), cache_rules);

    Ok(response)
}

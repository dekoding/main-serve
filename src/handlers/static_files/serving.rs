use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::Path;

use axum::body::Body;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use http::header;
use image::ImageFormat;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

use crate::config::types::ImageResizeConfig;
use crate::config::types::StaticFilesConfig;
use crate::config::types::StreamingConfig;
use crate::error::AppError;
use crate::handlers::static_files::routing::StaticGetContext;
use crate::handlers::static_files::utils::{apply_static_headers, mime_from_path};
use crate::server::AppState;
use crate::storage::Storage;

/// Handle GET requests - serve files with optional image resize or streaming.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if path traversal is detected.
/// Returns `AppError::NotFound` if the file is not found.
pub async fn handle_static_get(
    state: State<AppState>,
    ctx: StaticGetContext<'_>,
) -> Result<Response, AppError> {
    let storage = &*state.storage;
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

    // If path doesn't point to a file, check for spa_fallback
    // This handles both non-existent files and directories without index
    let file_exists = resolved_meta.as_ref().is_some_and(|m| m.is_file);
    if !file_exists && ctx.config.spa_fallback {
        return serve_spa_fallback(storage, ctx.root, &ctx.config.index, ctx.config).await;
    }

    if !file_exists {
        return Err(AppError::NotFound(format!(
            "File not found: {0}",
            ctx.request_path
        )));
    }

    // Check if user_scope is enabled.
    if let Some(user_scope) = &ctx.config.user_scope
        && user_scope.enabled
    {
        let query_params = ctx.query.map(|q| q.0.clone()).unwrap_or_default();
        let auth_info = crate::handlers::static_files::routing::extract_auth_info(
            &state,
            ctx.endpoint,
            ctx.headers,
            &query_params,
        )
        .await?;

        if let Some(required_role) = &user_scope.required_role {
            match &auth_info.role {
                Some(role) if role == required_role => {}
                _ => {
                    return Err(AppError::Forbidden(
                        "Authentication required to access user-scoped files".to_string(),
                    ));
                }
            }
        }

        // Construct user-specific path.
        // Construct user-specific path.
        let user_id = auth_info.subject.as_str();
        let pattern = &user_scope.directory_pattern;

        // Handle root directory exposure when expose_root is enabled.
        let user_resolved = if user_scope.expose_root {
            // When expose_root is true, allow access to both root files and user files.
            // Check if the relative path contains the user_id (user's file) or is at root level.
            let is_user_file = ctx.relative.split('/').any(|seg| seg == user_id);

            if is_user_file {
                // User is accessing their own file (path contains user_id)
                format!("{}/{user_id}/{1}", pattern, ctx.relative).replace("//", "/")
            } else {
                // Accessing root directory or subdirectory at root level
                ctx.relative.to_string()
            }
        } else {
            // Root not exposed - always use user-specific path
            format!("{}/{user_id}/{1}", pattern, ctx.relative).replace("//", "/")
        };

        let final_resolved = ctx.root.join(&user_resolved);

        // Canonicalize and check for path traversal.
        if let (Ok(canonical), Ok(root_canonical)) = (
            storage.canonicalize(&final_resolved).await,
            storage.canonicalize(ctx.root).await,
        ) && !canonical.starts_with(&root_canonical)
        {
            return Err(AppError::Forbidden("Path traversal denied".to_string()));
        }

        let meta = storage
            .metadata(&final_resolved)
            .await
            .map_err(|_| AppError::NotFound(format!("File not found: {0}", ctx.request_path)))?;
        if !meta.is_file {
            return Err(AppError::NotFound(format!(
                "File not found: {0}",
                ctx.request_path
            )));
        }

        return serve_file(
            storage,
            &final_resolved,
            ctx.config,
            Some(ctx.query.map(|q| q.0.clone()).unwrap_or_default()),
            ctx.headers,
        )
        .await;
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

    // Check if Range header is present for partial content.
    if let Some(range_header) = headers.get(header::RANGE)
        && let Ok(range_str) = range_header.to_str()
        && let Some(streaming_config) = &config.streaming
        && streaming_config.enabled
        && file_size > streaming_config.threshold
        && let Some(partial_response) = handle_range_request(
            storage,
            path,
            range_str,
            &content_type,
            config.cache_max_age,
            streaming_config,
        )
        .await?
    {
        return Ok(partial_response);
    }

    // Check if image resize is requested.
    if let Some(image_config) = &config.image_resize
        && image_config.enabled
        && is_image_path(path)
        && let Some(resized) =
            handle_image_resize(storage, path, query_params.as_ref(), image_config).await?
    {
        return Ok(resized);
    }

    // Check if streaming is enabled.
    if let Some(streaming_config) = &config.streaming
        && streaming_config.enabled
        && file_size > streaming_config.threshold
    {
        return serve_file_streaming(
            storage,
            path,
            &content_type,
            config.cache_max_age,
            streaming_config,
        )
        .await;
    }

    // Default: read entire file into memory.
    let contents = storage
        .read(path)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let mut response = (StatusCode::OK, contents).into_response();
    apply_static_headers(&mut response, &content_type, config.cache_max_age);
    Ok(response)
}

/// Serve a file using streaming response.
pub async fn serve_file_streaming(
    storage: &dyn Storage,
    path: &Path,
    content_type: &str,
    cache_max_age: u64,
    config: &StreamingConfig,
) -> Result<Response, AppError> {
    let file = storage
        .open(path)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let reader_stream = ReaderStream::with_capacity(file, config.buffer_size);
    let body = Body::from_stream(reader_stream);

    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str(&format!("public, max-age={cache_max_age}"))
            .unwrap_or(HeaderValue::from_static("public, max-age=3600")),
    );

    Ok(response)
}

/// Handle Range header for partial content (206).
///
/// Returns None if no Range header is present, or if the range cannot be satisfied.
async fn handle_range_request(
    storage: &dyn Storage,
    path: &Path,
    range_header: &str,
    content_type: &str,
    cache_max_age: u64,
    streaming_config: &StreamingConfig,
) -> Result<Option<Response>, AppError> {
    // Parse Range: bytes=START-END
    let range_value = range_header
        .strip_prefix("bytes=")
        .ok_or_else(|| AppError::BadRequest("Invalid Range header".to_string()))?;

    let (start_str, end_str) = range_value
        .split_once('-')
        .ok_or_else(|| AppError::BadRequest("Invalid Range format".to_string()))?;

    let file_size = storage
        .size(path)
        .await
        .map_err(|_| AppError::NotFound("File not found".to_string()))?;

    // Parse start position
    let start: u64 = start_str
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid Range start".to_string()))?;

    // Parse end position (optional - if empty, use file_size - 1)
    let end: Option<u64> = if end_str.is_empty() {
        Some(file_size - 1)
    } else {
        end_str.parse().ok()
    };

    let end = end.unwrap_or(file_size - 1).min(file_size - 1);
    let content_length = end.saturating_add(1).saturating_sub(start);

    // Validate range
    if start >= file_size || start > end {
        return Ok(Some(build_range_not_satisfiable_response(
            file_size,
            cache_max_age,
        )));
    }

    // Open file and seek to start position
    let mut file = storage
        .open(path)
        .await
        .map_err(|_| AppError::NotFound("File not found".to_string()))?;

    file.seek(SeekFrom::Start(start))
        .await
        .map_err(|_| AppError::Internal("Failed to seek file".to_string()))?;

    // Create streaming reader with limited content length
    let reader_stream =
        ReaderStream::with_capacity(file.take(content_length), streaming_config.buffer_size);
    let body = Body::from_stream(reader_stream);

    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::PARTIAL_CONTENT;

    // Set required headers for partial content
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
        HeaderValue::from_str(&format!("public, max-age={cache_max_age}"))
            .unwrap_or(HeaderValue::from_static("public, max-age=3600")),
    );

    Ok(Some(response))
}

/// Build response for range not satisfiable (416).
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
pub async fn handle_image_resize(
    storage: &dyn Storage,
    path: &Path,
    query_params: Option<&HashMap<String, String>>,
    config: &ImageResizeConfig,
) -> Result<Option<Response>, AppError> {
    let empty_map = HashMap::new();
    let query = query_params.unwrap_or(&empty_map);

    let width = query.get("w").and_then(|w| w.parse().ok());
    let height = query.get("h").and_then(|h| h.parse().ok());
    let fit = query.get("fit").map(|f| f.as_str());
    let output_format = query.get("format").map(|f| f.as_str());

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
            let ratio = img.width() as f32 / img.height() as f32;
            let h_ratio = h as f32 / w as f32;
            if ratio > h_ratio {
                let new_h = (w as f32 / ratio) as u32;
                (Some(w), Some(new_h))
            } else {
                let new_w = (h as f32 * ratio) as u32;
                (Some(new_w), Some(h))
            }
        }
        (Some(w), Some(h), _) => {
            let max_dim = config.max_dimension as u32;
            (Some(w.clamp(1, max_dim)), Some(h.clamp(1, max_dim)))
        }
        (None, None, None) => (None, None),
        (None, None, Some(_)) => (None, None),
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
    apply_static_headers(&mut response, content_type, 0);
    Ok(Some(response))
}

// Check if a path is likely an image.
pub fn is_image_path(path: &Path) -> bool {
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

/// Serve the index file as a SPA fallback.
pub async fn serve_spa_fallback(
    storage: &dyn Storage,
    root: &Path,
    index: &str,
    config: &StaticFilesConfig,
) -> Result<Response, AppError> {
    let index_path = root.join(index);
    let contents = storage
        .read(&index_path)
        .await
        .map_err(|_| AppError::NotFound(format!("Index file not found: {index}")))?;
    let mut response = (StatusCode::OK, contents).into_response();
    apply_static_headers(
        &mut response,
        &mime_from_path(&index_path),
        config.cache_max_age,
    );
    Ok(response)
}

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
use tokio_util::io::ReaderStream;

use crate::config::types::ImageResizeConfig;
use crate::config::types::StaticFilesConfig;
use crate::config::types::StreamingConfig;
use crate::error::AppError;
use crate::handlers::static_files::routing::StaticGetContext;
use crate::handlers::static_files::utils::{
    apply_static_headers, mime_from_path,
};
use crate::server::AppState;

/// Handle GET requests - serve files with optional image resize or streaming.
pub async fn handle_static_get(
    state: State<AppState>,
    ctx: StaticGetContext<'_>,
) -> Result<Response, AppError> {
    let _ = state;
    let resolved = if ctx.relative.is_empty() {
        ctx.root.to_path_buf()
    } else {
        let r = ctx.root.join(ctx.relative);
        if let (Ok(canonical), Ok(root_canonical)) = (
            tokio::fs::canonicalize(&r).await,
            tokio::fs::canonicalize(ctx.root).await,
        ) && !canonical.starts_with(&root_canonical)
        {
            return Err(AppError::Forbidden("Path traversal denied".to_string()));
        }
        r
    };

    let resolved_meta = tokio::fs::metadata(&resolved).await.ok();
    if resolved_meta
        .as_ref()
        .is_some_and(std::fs::Metadata::is_dir)
    {
        let index_path = resolved.join(&ctx.config.index);
        let index_exists = tokio::fs::metadata(&index_path)
            .await
            .is_ok_and(|m| m.is_file());
        if index_exists {
            return serve_file(&index_path, ctx.config, None).await;
        }
        if ctx.config.directory_listing {
            return crate::handlers::static_files::directory::generate_directory_listing(
                &resolved,
                ctx.request_path,
            )
            .await;
        }
        if ctx.config.spa_fallback {
            return serve_spa_fallback(ctx.root, &ctx.config.index, ctx.config).await;
        }
        return Err(AppError::NotFound(format!(
            "File not found: {0}",
            ctx.request_path
        )));
    }

    // Check if user_scope is enabled.
    if let Some(user_scope) = &ctx.config.user_scope
        && user_scope.enabled {
            let query_params = ctx.query.0.as_ref().cloned().unwrap_or_default();
            let auth_info =
                crate::handlers::static_files::routing::extract_auth_info(
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
            let user_id = auth_info.subject.as_str();
            let pattern = &user_scope.directory_pattern;
            let user_path = format!("{}/{user_id}/{1}", pattern, ctx.relative).replace("//", "/");
            let user_resolved = ctx.root.join(&user_path);
            if let (Ok(canonical), Ok(root_canonical)) = (
                tokio::fs::canonicalize(&user_resolved).await,
                tokio::fs::canonicalize(ctx.root).await,
            ) && !canonical.starts_with(&root_canonical)
            {
                return Err(AppError::Forbidden("Path traversal denied".to_string()));
            }

            let meta = tokio::fs::metadata(&user_resolved)
                .await
                .map_err(|_| AppError::NotFound(format!("File not found: {0}", ctx.request_path)))?;
            if !meta.is_file() {
                return Err(AppError::NotFound(format!(
                    "File not found: {0}",
                    ctx.request_path
                )));
            }

            return serve_file(&user_resolved, ctx.config, None).await;
        }

    serve_file(&resolved, ctx.config, ctx.query.0.clone()).await
}

/// Serve a single file with optional image resize or streaming.
pub async fn serve_file(
    path: &Path,
    config: &StaticFilesConfig,
    query_params: Option<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let file_size = meta.len();
    let content_type = mime_from_path(path);

    // Check if image resize is requested.
    if let Some(image_config) = &config.image_resize
        && image_config.enabled
            && is_image_path(path)
            && let Some(resized) = handle_image_resize(path, query_params.as_ref(), image_config).await?
        {
            return Ok(resized);
        }

    // Check if streaming is enabled.
    if let Some(streaming_config) = &config.streaming
        && streaming_config.enabled && file_size > streaming_config.threshold {
            return serve_file_streaming(path, &content_type, config.cache_max_age, streaming_config)
                .await;
        }

    // Default: read entire file into memory.
    let contents = tokio::fs::read(path)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let mut response = (StatusCode::OK, contents).into_response();
    apply_static_headers(&mut response, &content_type, config.cache_max_age);
    Ok(response)
}

/// Serve a file using streaming response.
pub async fn serve_file_streaming(
    path: &Path,
    content_type: &str,
    cache_max_age: u64,
    config: &StreamingConfig,
) -> Result<Response, AppError> {
    let file = tokio::fs::File::open(path)
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

/// Handle image resize if parameters are present.
pub async fn handle_image_resize(
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
    let image_data = tokio::fs::read(path)
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
    root: &Path,
    index: &str,
    config: &StaticFilesConfig,
) -> Result<Response, AppError> {
    let index_path = root.join(index);
    let contents = tokio::fs::read(&index_path)
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
/// Media resize and thumbnail handlers.
use std::path::Path;

use axum::response::Response;

use crate::config::types::{EndpointConfig, MediaConfig};
use crate::db::query::select_one::build_select_file_path;
use crate::error::AppError;
use crate::handlers::common::helpers::extract_file_path;
use crate::handlers::common::resize::{build_resize_response, parse_resize_params, resize_image};
use crate::storage::Storage;

/// Handle media resize.
///
/// # Errors
///
/// Returns an `AppError::MethodNotAllowed` if image resize is not enabled.
pub async fn handle_media_resize<S: std::hash::BuildHasher + Send + Sync>(
    storage: &dyn Storage,
    root: &Path,
    id: &str,
    endpoint: &EndpointConfig,
    config: &MediaConfig,
    query_params: &std::collections::HashMap<String, String, S>,
    pool: &crate::db::pool::DatabasePool,
) -> Result<Response, AppError> {
    let image_resize = config
        .image_resize
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed {
            message: "Image resize is not enabled".to_string(),
            allowed: endpoint.methods.clone(),
        })?;

    if !image_resize.enabled {
        return Err(AppError::MethodNotAllowed {
            message: "Image resize is not enabled".to_string(),
            allowed: endpoint.methods.clone(),
        });
    }

    // Look up file_path from DB
    let built = build_select_file_path(&config.table, pool.driver());
    let row = pool.fetch_optional_json(&built.sql, &[id.into()]).await?;

    let file_path = extract_file_path(row, id)?;

    let resolved_path = root.join(&file_path);

    let image_data = storage
        .read(&resolved_path)
        .await
        .map_err(|_| AppError::NotFound(format!("Media file not found: {file_path}")))?;

    let params = parse_resize_params(query_params);

    // Determine default format from file content
    let default_format = image::guess_format(&image_data).unwrap_or(image::ImageFormat::Png);

    let (resized_bytes, content_type) = resize_image(
        &image_data,
        &params,
        default_format,
        u32::try_from(image_resize.max_dimension).unwrap_or(u32::MAX),
    )?;

    Ok(build_resize_response(
        resized_bytes,
        content_type,
        Some(86400),
    ))
}

/// Handle a media thumbnail request for a specific image.
///
/// Looks up the image in the database, reads it from storage, applies a
/// thumbnail-sized resize (150px or the configured thumbnail style size),
/// and returns the resized image with a 24-hour cache header.
///
/// # Errors
///
/// Returns `AppError::MethodNotAllowed` if image resize is not enabled.
/// Returns `AppError::NotFound` if the image file is not found.
/// Returns `AppError::Internal` on encoding errors.
pub async fn handle_media_thumbnail(
    storage: &dyn Storage,
    root: &Path,
    id: &str,
    endpoint: &EndpointConfig,
    config: &MediaConfig,
    pool: &crate::db::pool::DatabasePool,
) -> Result<Response, AppError> {
    let image_resize = config
        .image_resize
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed {
            message: "Image resize is not enabled".to_string(),
            allowed: endpoint.methods.clone(),
        })?;

    let default_size = image_resize
        .styles
        .iter()
        .find(|s| s.name == "thumbnail")
        .map_or(150, |s| s.max_width.min(s.max_height));

    let mut params = std::collections::HashMap::new();
    params.insert("w".to_string(), default_size.to_string());
    params.insert("h".to_string(), default_size.to_string());

    handle_media_resize(storage, root, id, endpoint, config, &params, pool).await
}

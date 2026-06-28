/// Media resize and thumbnail handlers.
use std::collections::HashMap;
use std::path::Path;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use http::HeaderValue;
use http::header;

use crate::config::types::MediaConfig;
use crate::db::migration::quote_object_name;
use crate::error::AppError;
use crate::storage::Storage;

/// Handle media resize.
pub async fn handle_media_resize(
    storage: &dyn Storage,
    root: &Path,
    id: &str,
    config: &MediaConfig,
    query_params: &HashMap<String, String>,
    pool: &crate::db::pool::DatabasePool,
) -> Result<Response, AppError> {
    let image_resize = config
        .image_resize
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Image resize is not enabled".to_string()))?;

    if !image_resize.enabled {
        return Err(AppError::MethodNotAllowed(
            "Image resize is not enabled".to_string(),
        ));
    }

    // Look up file_path from DB
    let driver = pool.driver();
    let sql = format!(
        "SELECT file_path FROM {} WHERE id = $1",
        quote_object_name(&config.table, driver)
    );
    let row = pool.fetch_optional_json(&sql, &[id.into()]).await?;

    let file_path: String = row
        .and_then(|r| {
            r.get("file_path")
                .and_then(|v| v.as_str().map(String::from))
        })
        .unwrap_or_else(|| format!("media/{}", id));

    let resolved_path = root.join(&file_path);

    let image_data = storage
        .read(&resolved_path)
        .await
        .map_err(|_| AppError::NotFound(format!("Media file not found: {}", file_path)))?;

    let format = image::ImageFormat::from_extension("jpg").unwrap_or(image::ImageFormat::Png);
    let img = image::load_from_memory(&image_data)
        .map_err(|_| AppError::BadRequest("Invalid image data".to_string()))?;

    let width = query_params.get("w").and_then(|w| w.parse().ok());
    let height = query_params.get("h").and_then(|h| h.parse().ok());
    let fit = query_params.get("fit").map(std::string::String::as_str);

    let (target_width, target_height) = match (width, height, fit) {
        (Some(w), None, _) => (Some(w), None),
        (None, Some(h), _) => (None, Some(h)),
        (Some(w), Some(h), Some("cover")) => {
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
            let max_dim = u32::try_from(image_resize.max_dimension).unwrap_or(u32::MAX);
            (Some(w.clamp(1, max_dim)), Some(h.clamp(1, max_dim)))
        }
        _ => (Some(img.width()), Some(img.height())),
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

    let output_format = query_params
        .get("format")
        .and_then(image::ImageFormat::from_extension)
        .unwrap_or(format);

    let mut output_bytes = Vec::new();
    resized
        .write_to(&mut std::io::Cursor::new(&mut output_bytes), output_format)
        .map_err(|_| AppError::Internal("Failed to encode resized image".to_string()))?;

    let content_type = match output_format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::Gif => "image/gif",
        image::ImageFormat::WebP => "image/webp",
        _ => "application/octet-stream",
    };

    let mut response = (StatusCode::OK, output_bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str("public, max-age=86400")
            .unwrap_or(HeaderValue::from_static("public, max-age=3600")),
    );

    Ok(response)
}

pub async fn handle_media_thumbnail(
    storage: &dyn Storage,
    root: &Path,
    id: &str,
    config: &MediaConfig,
    pool: &crate::db::pool::DatabasePool,
) -> Result<Response, AppError> {
    let image_resize = config
        .image_resize
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Image resize is not enabled".to_string()))?;

    let default_size = image_resize
        .styles
        .iter()
        .find(|s| s.name == "thumbnail")
        .map(|s| s.max_width.min(s.max_height))
        .unwrap_or(150);

    let mut params = HashMap::new();
    params.insert("w".to_string(), default_size.to_string());
    params.insert("h".to_string(), default_size.to_string());

    handle_media_resize(storage, root, id, config, &params, pool).await
}

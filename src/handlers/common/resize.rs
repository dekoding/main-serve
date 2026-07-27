/// Shared image resize logic used by `static_files` and media handlers.
///
/// Both modules needed identical implementations for:
/// - Parsing resize query parameters (w, h, fit, format)
/// - Calculating target dimensions
/// - Resizing with Lanczos3 filter
/// - Encoding to output format
///
/// This module centralizes that logic so both callers can focus on
/// their specific response-building responsibilities.
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::error::AppError;
use crate::handlers::common::utils::{apply_cache_control, apply_content_type};

/// Query parameters for image resizing.
pub struct ResizeParams<'a> {
    /// Target width in pixels (None = preserve aspect ratio from height).
    pub width: Option<u32>,
    /// Target height in pixels (None = preserve aspect ratio from width).
    pub height: Option<u32>,
    /// Resize fit mode (e.g. "cover", "contain", `` `scale_down` ``).
    pub fit: Option<&'a str>,
    /// Output image format (None = use source format).
    pub output_format: Option<image::ImageFormat>,
}

/// Parse resize query parameters from a query map.
pub fn parse_resize_params<S: std::hash::BuildHasher>(
    query_params: &std::collections::HashMap<String, String, S>,
) -> ResizeParams<'_> {
    let width = query_params.get("w").and_then(|w| w.parse().ok());
    let height = query_params.get("h").and_then(|h| h.parse().ok());
    let fit = query_params.get("fit").map(std::string::String::as_str);
    let output_format = query_params
        .get("format")
        .and_then(image::ImageFormat::from_extension);

    ResizeParams {
        width,
        height,
        fit,
        output_format,
    }
}

/// Calculate resized dimensions from source image and resize params.
///
/// Returns `(target_width, target_height)`. If no resizing is needed,
/// returns the original dimensions.
#[must_use]
pub fn calculate_target_dimensions(
    src_width: u32,
    src_height: u32,
    params: &ResizeParams,
    max_dimension: u32,
) -> (u32, u32) {
    let (target_width, target_height) = match (params.width, params.height, params.fit) {
        (Some(w), None, _) => (Some(w), None),
        (None, Some(h), _) => (None, Some(h)),
        (Some(w), Some(h), Some("cover")) => {
            let ratio = f64::from(src_width) / f64::from(src_height);
            let h_ratio = f64::from(h) / f64::from(w);
            if ratio > h_ratio {
                #[allow(clippy::cast_possible_truncation)]
                #[allow(clippy::cast_sign_loss)]
                let new_h = (f64::from(w) / ratio) as u32;
                (Some(w), Some(new_h))
            } else {
                #[allow(clippy::cast_possible_truncation)]
                #[allow(clippy::cast_sign_loss)]
                let new_w = (f64::from(h) * ratio) as u32;
                (Some(new_w), Some(h))
            }
        }
        (Some(w), Some(h), _) => (
            Some(w.clamp(1, max_dimension)),
            Some(h.clamp(1, max_dimension)),
        ),
        _ => (Some(src_width), Some(src_height)),
    };

    (
        target_width.unwrap_or(src_width),
        target_height.unwrap_or(src_height),
    )
}

/// Resize an image from memory using the given parameters.
///
/// Returns the resized image bytes and the content type for the output format.
///
/// # Errors
///
/// Returns `AppError::Internal` if image encoding fails.
pub fn resize_image(
    image_data: &[u8],
    params: &ResizeParams,
    default_format: image::ImageFormat,
    max_dimension: u32,
) -> Result<(Vec<u8>, &'static str), AppError> {
    let format = params.output_format.unwrap_or(default_format);
    let img = image::load_from_memory(image_data)
        .map_err(|_| AppError::BadRequest("Invalid image data".to_string()))?;

    let (target_w, target_h) =
        calculate_target_dimensions(img.width(), img.height(), params, max_dimension);

    let resized = img.resize(target_w, target_h, image::imageops::FilterType::Lanczos3);

    let mut output_bytes = Vec::new();
    resized
        .write_to(&mut std::io::Cursor::new(&mut output_bytes), format)
        .map_err(|_| AppError::Internal("Failed to encode resized image".to_string()))?;

    let content_type = match format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::Gif => "image/gif",
        image::ImageFormat::WebP => "image/webp",
        _ => "application/octet-stream",
    };

    Ok((output_bytes, content_type))
}

/// Build a response for resized image data.
///
/// Sets Content-Type and optional Cache-Control headers.
#[must_use]
pub fn build_resize_response(
    output_bytes: Vec<u8>,
    content_type: &str,
    cache_max_age: Option<u64>,
) -> Response {
    let mut response = (StatusCode::OK, output_bytes).into_response();

    apply_content_type(&mut response, content_type);

    if let Some(max_age) = cache_max_age {
        apply_cache_control(&mut response, max_age, None, &[]);
    }

    response
}

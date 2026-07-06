/// Shared helpers for ID extraction and image detection.
///
/// These utilities eliminate duplicated logic across media, file_store, and
/// static_files handlers.
use std::path::Path;

use crate::error::AppError;

/// Extract the last path segment as an ID string.
///
/// Works for paths like `/media/123`, `/_main-serve/media/trash/456`, etc.
#[must_use]
pub fn extract_id(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    segments.last().map(|s| s.to_string())
}

/// Extract the extension from a file path and check if it suggests an image file.
#[must_use]
pub fn is_image_path(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        is_image_extension(ext)
    } else {
        false
    }
}

/// Check if a file extension suggests an image file.
///
/// Supports common image formats: jpg, jpeg, png, gif, webp, bmp, svg, ico.
#[must_use]
pub fn is_image_extension(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" | "ico"
    )
}

/// Validate magic bytes for image files.
///
/// This provides an extra layer of security by checking the actual file
/// format rather than relying solely on file extension.
pub fn validate_image_magic_bytes(data: &[u8]) -> Result<(), AppError> {
    // PNG signature
    if data.len() >= 8 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
        return Ok(());
    }

    // JPEG signature: must be 0xFF 0xD8 0xFF (SOI + start of marker)
    if data.len() >= 3 && &data[0..3] == b"\xFF\xD8\xFF" {
        return Ok(());
    }

    // GIF signature
    if data.len() >= 6 && (&data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a") {
        return Ok(());
    }

    // BMP signature
    if data.len() >= 2 && &data[0..2] == b"BM" {
        return Ok(());
    }

    // WebP signature
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Ok(());
    }

    // SVG signature (XML-based)
    if data.len() >= 4 && data[0] == b'<' && data[1] == b'?' {
        return Ok(());
    }

    Err(AppError::BadRequest(
        "File does not appear to be a valid image based on magic bytes".to_string(),
    ))
}

/// Parse a multipart form and extract a single file's content and filename.
///
/// Iterates over multipart fields, accumulates chunks for the first field
/// with a filename, validates the content size, and optionally checks
/// image magic bytes.
///
/// # Arguments
///
/// * `multipart` - The multipart stream to parse.
/// * `max_size` - Maximum allowed file size in bytes.
/// * `validate_image_magic` - If true and the file has an image extension,
///   validates the magic bytes against known image signatures.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if the form is malformed, a field has no
/// name, or no file was provided.
/// Returns `AppError::PayloadTooLarge` if the file exceeds `max_size`.
/// Returns `AppError::BadRequest` if image magic byte validation fails.
pub async fn parse_multipart_file(
    multipart: &mut axum::extract::Multipart,
    max_size: u64,
    validate_image_magic: bool,
) -> Result<(Vec<u8>, String), AppError> {
    let mut file_content = Vec::new();
    let mut original_filename = String::new();
    let mut found_file = false;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to parse multipart form: {e}")))?
    {
        if field.name().is_none() {
            return Err(AppError::BadRequest("Form field missing name".to_string()));
        }

        if let Some(filename) = field.file_name() {
            original_filename = filename.to_string();
        } else {
            continue;
        }

        found_file = true;
        let mut field_bytes = Vec::new();
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| AppError::BadRequest(format!("Failed to read file content: {e}")))?
        {
            field_bytes.extend_from_slice(&chunk);
        }
        file_content = field_bytes;

        if file_content.len() as u64 > max_size {
            return Err(AppError::PayloadTooLarge(format!(
                "File size {} exceeds maximum allowed size {}",
                file_content.len(),
                max_size
            )));
        }

        if validate_image_magic
            && let Some(ext) = Path::new(&original_filename)
                .extension()
                .and_then(|e| e.to_str())
                && is_image_extension(ext)
            {
                validate_image_magic_bytes(&file_content)?;
            }
    }

    if !found_file {
        return Err(AppError::BadRequest(
            "No file provided in multipart form".to_string(),
        ));
    }

    Ok((file_content, original_filename))
}

pub async fn extract_file_path(
    row: Option<serde_json::Value>,
    id: &str,
) -> Result<String, AppError> {
    
    row
        .and_then(|r| {
            r.get("file_path")
                .and_then(|v| v.as_str().map(String::from))
        })
        .ok_or_else(|| AppError::NotFound(format!("Item with id '{}' not found", id)))
}

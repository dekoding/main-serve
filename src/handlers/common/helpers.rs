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

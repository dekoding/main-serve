/// Shared helpers for ID extraction and image detection.
///
/// These utilities eliminate duplicated logic across media, file_store, and
/// static_files handlers.
use std::path::Path;

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

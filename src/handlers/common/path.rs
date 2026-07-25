/// Shared path utilities for file storage operations.
///
/// These functions handle storage path construction, filename sanitization,
/// and subdirectory pattern expansion used by uploads across media and
/// static_files handlers.
use std::path::Path;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use uuid::Uuid;

use crate::error::AppError;
use crate::storage::Storage;

/// Generate a sanitized filename for uploads.
///
/// If the original filename is provided and has a valid extension,
/// generates a UUID-based filename to prevent collisions and overwrites.
/// Falls back to the relative path if no original filename is available.
pub fn generate_upload_filename(
    original_filename: &str,
    allowed_extensions: &[String],
) -> Result<String, AppError> {
    let extension = std::path::Path::new(original_filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if !extension.is_empty()
        && !allowed_extensions.is_empty()
        && !allowed_extensions.iter().any(|ext| ext == &extension)
    {
        return Err(AppError::BadRequest(format!(
            "File extension .{} is not allowed. Allowed: {}",
            extension,
            allowed_extensions.join(", ")
        )));
    }

    let uuid = Uuid::new_v4();

    if extension.is_empty() {
        Ok(uuid.to_string())
    } else {
        Ok(format!("{uuid}.{extension}"))
    }
}

/// Sanitize filename to prevent path traversal and special characters.
///
/// When `is_upload` is true (file upload), generates a UUID-based filename
/// to prevent overwrites and collisions. When false (GET requests), returns
/// the sanitized name for path resolution.
pub fn sanitize_filename(name: &str, is_upload: bool) -> Result<String, AppError> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(AppError::BadRequest("Invalid filename".to_string()));
    }

    let sanitized: String = name
        .chars()
        .filter(|c| !c.is_ascii_control() && *c != '\0')
        .collect();

    if sanitized.is_empty() {
        return Err(AppError::BadRequest("Filename cannot be empty".to_string()));
    }

    if is_upload {
        let uuid = Uuid::new_v4();
        return Ok(format!("{uuid}.{sanitized}").to_lowercase());
    }

    Ok(sanitized.to_lowercase())
}

/// Build storage path with subdirectory pattern.
pub fn build_storage_path(
    root: &Path,
    filename: &str,
    user_id: &str,
    subdirectory_pattern: &Option<String>,
) -> Result<std::path::PathBuf, AppError> {
    let final_path = if let Some(pattern) = subdirectory_pattern {
        let now = chrono::Utc::now();
        let expanded = pattern
            .replace("{user_id}", user_id)
            .replace("{year}", &now.format("%Y").to_string())
            .replace("{month}", &now.format("%m").to_string())
            .replace("{day}", &now.format("%d").to_string())
            .replace("{uuid}", &Uuid::new_v4().to_string());

        if expanded.split('/').any(|seg| seg == "..") {
            return Err(AppError::Forbidden(
                "Invalid subdirectory pattern".to_string(),
            ));
        }
        root.join(&expanded).join(filename)
    } else {
        root.join(filename)
    };

    if final_path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(AppError::Forbidden("Invalid path".to_string()));
    }

    Ok(final_path)
}

/// Prevents path traversal
pub fn validate_path_within(path: &Path, root: &Path) -> Result<(), AppError> {
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
        || path.strip_prefix(root).is_err()
    {
        return Err(AppError::Forbidden("Path traversal denied".to_string()));
    }
    Ok(())
}

/// Extract the relative file path from a request URI and endpoint path.
///
/// Strips the endpoint's base path prefix (handling `/*` and `{*rest}`
/// wildcards) from the request path and returns the remaining segment.
pub fn extract_relative_path(request_path: &str, endpoint_path: &str) -> Result<String, AppError> {
    let ep_path = endpoint_path
        .trim_end_matches("/*")
        .trim_end_matches("{*rest}");
    let stripped = request_path
        .strip_prefix(ep_path)
        .unwrap_or(request_path)
        .trim_start_matches('/');

    if !stripped.is_empty() {
        let decoded = percent_encoding::percent_decode_str(stripped)
            .decode_utf8()
            .map_err(|_| AppError::BadRequest("Invalid UTF-8 in path".to_string()))?;
        if decoded.split('/').any(|seg| seg == ".." || seg == ".") {
            return Err(AppError::Forbidden("Path traversal denied".to_string()));
        }
    }
    Ok(stripped.to_string())
}

/// Build the HTTP response for a successful file upload.
///
/// Returns a 201 CREATED response with JSON body containing file metadata.
pub async fn build_upload_response(
    storage_path: &Path,
    root: &Path,
    file_content: &[u8],
    storage: Arc<dyn Storage>,
    sanitized_filename: &str,
) -> Result<axum::http::Response<axum::body::Body>, AppError> {
    let metadata = storage
        .metadata(storage_path)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to read file metadata: {e}")))?;

    let created = metadata.created.map_or_else(
        || chrono::Utc::now().to_rfc3339(),
        |t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339(),
    );

    let modified = metadata
        .modified
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
        .unwrap_or(created.clone());

    let mime_type = crate::config::types::mime_from_path(storage_path);

    let relative_path = storage_path.strip_prefix(root).map_or_else(
        |_| format!("/{sanitized_filename}"),
        |p| format!("/{}", p.to_string_lossy()),
    );

    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({
            "success": true,
            "path": relative_path,
            "name": sanitized_filename,
            "size": file_content.len(),
            "type": mime_type,
            "created": created,
            "modified": modified,
            "message": "File uploaded successfully"
        })),
    )
        .into_response())
}

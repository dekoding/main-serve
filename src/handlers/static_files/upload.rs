use std::collections::HashMap;
use std::path::Path;

use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use http::header::CONTENT_LENGTH;
use uuid::Uuid;

use crate::config::types::StaticFilesConfig;
use crate::error::AppError;
use crate::handlers::static_files::routing::{check_upload_role, extract_auth_info};
use crate::handlers::static_files::utils::mime_from_path;
use crate::server::state::AppState;

/// Handle file upload (POST/PUT/PATCH).
pub async fn handle_file_upload(
    mut multipart: axum::extract::Multipart,
    state: State<AppState>,
    endpoint: &crate::config::types::EndpointConfig,
    config: &StaticFilesConfig,
    _relative: &str,
    root: &Path,
    headers: &axum::http::HeaderMap,
) -> Result<axum::http::Response<Body>, AppError> {
    let storage = &state.storage;
    let upload_config = config
        .upload
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("File management is not enabled".to_string()))?;

    if !upload_config.enabled {
        return Err(AppError::MethodNotAllowed(
            "File management is not enabled".to_string(),
        ));
    }

    // Extract auth info.
    let query_params: HashMap<String, String> = endpoint
        .crud
        .as_ref()
        .map(|c| {
            c.filtering
                .allowed_fields
                .iter()
                .map(|field| (field.clone(), field.clone()))
                .collect()
        })
        .unwrap_or_default();
    let auth_info = extract_auth_info(&state, endpoint, headers, &query_params).await?;
    check_upload_role(&auth_info, upload_config)?;

    let user_id = &auth_info.subject;

    // Validate file size from Content-Length header if available.
    if let Some(content_length) = headers.get(CONTENT_LENGTH)
        && let Ok(size) = content_length.to_str().unwrap_or("0").parse::<u64>()
        && size > upload_config.max_size
    {
        return Err(AppError::PayloadTooLarge(format!(
            "File size {} exceeds maximum allowed size {}",
            size, upload_config.max_size
        )));
    }

    // Parse multipart form data and extract the file.
    let mut file_content = Vec::new();
    let mut found_file = false;
    let mut original_filename = String::new();

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to parse multipart form: {}", e)))?
    {
        // Validate field name
        if let Some(field_name) = field.name() {
            // Accept common file field names
            if field_name != "file" && field_name != "files" && field_name != "upload" {
                // Allow other field names but log a warning
                // This provides flexibility for different client implementations
            }
        } else {
            return Err(AppError::BadRequest("Form field missing name".to_string()));
        }

        // Extract original filename from multipart metadata
        if let Some(filename) = field.file_name() {
            original_filename = filename.to_string();
        }

        found_file = true;
        // Read file content into buffer
        let mut field_bytes = Vec::new();
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| AppError::BadRequest(format!("Failed to read file content: {}", e)))?
        {
            field_bytes.extend_from_slice(&chunk);
        }
        file_content = field_bytes;

        // Validate file size from actual content
        if file_content.len() as u64 > upload_config.max_size {
            return Err(AppError::PayloadTooLarge(format!(
                "File size {} exceeds maximum allowed size {}",
                file_content.len(),
                upload_config.max_size
            )));
        }

        // Optional: Validate image magic bytes if it's an image
        if let Some(ext) = std::path::Path::new(&original_filename)
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

    // Generate sanitized filename
    let sanitized_filename =
        generate_upload_filename(&original_filename, &upload_config.allowed_extensions)?;

    // Build storage path.
    let storage_path = build_storage_path(
        root,
        &sanitized_filename,
        user_id,
        &upload_config.create_subdirectory,
    )?;

    // Ensure parent directories exist.
    if let Some(parent) = storage_path.parent() {
        storage
            .create_dir_all(parent)
            .await
            .map_err(|e| AppError::FileOperation(format!("Failed to create directory: {e}")))?;
    }

    // Validate extension.
    let file_extension = sanitized_filename
        .rsplit('.')
        .next()
        .map(|ext: &str| ext.to_lowercase())
        .unwrap_or_default();

    if !upload_config.allowed_extensions.is_empty()
        && !upload_config.allowed_extensions.contains(&file_extension)
    {
        return Err(AppError::BadRequest(format!(
            "File extension .{} is not allowed",
            file_extension
        )));
    }

    // Check if file already exists at the destination path.
    if storage.exists(&storage_path).await {
        return Err(AppError::FileOperation(format!(
            "A file already exists at the destination path: {}",
            storage_path.display()
        )));
    }

    // Write file content to disk.
    storage
        .write(&storage_path, &file_content)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to write file: {}", e)))?;

    // Get file metadata for response
    let metadata = storage
        .metadata(&storage_path)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to read file metadata: {}", e)))?;

    let created = metadata
        .created
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    let modified = metadata
        .modified
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
        .unwrap_or(created.clone());

    let mime_type = mime_from_path(&storage_path);

    // Return success response with detailed file metadata.
    // Return the path relative to root for client use
    let relative_path = storage_path
        .strip_prefix(root)
        .map(|p| format!("/{}", p.to_string_lossy()))
        .unwrap_or_else(|_| format!("/{}", sanitized_filename));

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

/// Generate a sanitized filename for uploads.
///
/// If the original filename is provided and has a valid extension,
/// generates a UUID-based filename to prevent collisions and overwrites.
/// Falls back to the relative path if no original filename is available.
fn generate_upload_filename(
    original_filename: &str,
    allowed_extensions: &[String],
) -> Result<String, AppError> {
    let extension = std::path::Path::new(original_filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Validate extension if we have one
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

    // Generate UUID-based filename to prevent collisions and overwrites
    let uuid = Uuid::new_v4();

    if extension.is_empty() {
        // No extension in original filename, use UUID only
        Ok(uuid.to_string())
    } else {
        // Include extension: UUID.extension
        Ok(format!("{}.{}", uuid, extension))
    }
}

/// Sanitize filename to prevent path traversal and special characters.
///
/// When `is_upload` is true (file upload), generates a UUID-based filename
/// to prevent overwrites and collisions. When false (GET requests), returns
/// the sanitized name for path resolution.
pub fn sanitize_filename(name: &str, is_upload: bool) -> Result<String, AppError> {
    // Reject any path separators or traversal attempts.
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(AppError::BadRequest("Invalid filename".to_string()));
    }

    // Remove any null bytes or control characters.
    let sanitized: String = name
        .chars()
        .filter(|c| !c.is_ascii_control() && *c != '\0')
        .collect();

    if sanitized.is_empty() {
        return Err(AppError::BadRequest("Filename cannot be empty".to_string()));
    }

    // For uploads, use UUID to prevent collisions and overwrites.
    // This also prevents directory traversal via filename manipulation.
    if is_upload {
        let uuid = Uuid::new_v4();
        return Ok(format!("{}.{}", uuid, sanitized).to_lowercase());
    }

    // For GET requests, just return the sanitized name.
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
        let expanded = expand_subdirectory_pattern(pattern, user_id)?;
        root.join(&expanded).join(filename)
    } else {
        root.join(filename)
    };

    // Prevent path traversal even after pattern expansion.
    if final_path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(AppError::Forbidden("Invalid path".to_string()));
    }

    Ok(final_path)
}

/// Expand subdirectory pattern with placeholders.
pub fn expand_subdirectory_pattern(pattern: &str, user_id: &str) -> Result<String, AppError> {
    let now = chrono::Utc::now();
    let expanded = pattern
        .replace("{user_id}", user_id)
        .replace("{year}", &now.format("%Y").to_string())
        .replace("{month}", &now.format("%m").to_string())
        .replace("{day}", &now.format("%d").to_string())
        .replace("{uuid}", &Uuid::new_v4().to_string());

    // Check for path traversal in expanded path.
    if expanded.split('/').any(|seg| seg == "..") {
        return Err(AppError::Forbidden(
            "Invalid subdirectory pattern".to_string(),
        ));
    }

    Ok(expanded)
}

/// Check if a file extension suggests an image file.
fn is_image_extension(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" | "ico"
    )
}

/// Validate magic bytes for image files.
///
/// This provides an extra layer of security by checking the actual file
/// format rather than relying solely on file extension.
fn validate_image_magic_bytes(data: &[u8]) -> Result<(), AppError> {
    // PNG signature
    if data.len() >= 8 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
        return Ok(());
    }

    // JPEG signature (start of file)
    if data.len() >= 3 && &data[0..2] == b"\xFF\xD8" && data[2] != 0xFF {
        return Ok(());
    }

    // GIF signature
    if data.len() >= 6 && &data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a" {
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

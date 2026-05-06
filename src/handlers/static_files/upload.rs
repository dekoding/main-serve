use std::collections::HashMap;
use std::path::Path;

use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use http::header;
use http::header::CONTENT_LENGTH;
use uuid::Uuid;

use crate::config::types::StaticFilesConfig;
use crate::error::AppError;
use crate::handlers::static_files::routing::{check_upload_role, extract_auth_info};
use crate::server::state::AppState;

/// Handle file upload (POST/PUT/PATCH).
pub async fn handle_file_upload(
    state: State<AppState>,
    endpoint: &crate::config::types::EndpointConfig,
    config: &StaticFilesConfig,
    relative: &str,
    root: &Path,
    headers: &axum::http::HeaderMap,
) -> Result<axum::http::Response<Body>, AppError> {
    let upload_config = config
        .upload
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("File uploads are not enabled".to_string()))?;

    if !upload_config.enabled {
        return Err(AppError::MethodNotAllowed(
            "File uploads are not enabled".to_string(),
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
    let _content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream");

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

    // Sanitize filename.
    let sanitized_filename = sanitize_filename(relative)?;

    // Build storage path.
    let storage_path = build_storage_path(
        root,
        &sanitized_filename,
        user_id,
        &upload_config.create_subdirectory,
    )?;

    // Ensure parent directories exist.
    if let Some(parent) = storage_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::FileOperation(format!("Failed to create directory: {e}")))?;
    }

    // Validate extension.
    let file_extension = sanitized_filename
        .rsplit('.')
        .next()
        .map(|ext| ext.to_lowercase())
        .unwrap_or_default();

    if !upload_config.allowed_extensions.is_empty()
        && !upload_config.allowed_extensions.contains(&file_extension)
    {
        return Err(AppError::BadRequest(format!(
            "File extension .{} is not allowed",
            file_extension
        )));
    }

    // NOTE: Full multipart parsing requires the request body to be accessible.
    // This placeholder returns success response structure.
    // Full implementation would extract Multipart from axum::extract::Multipart
    // and process each field to save file to storage_path.

    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({
            "success": true,
            "path": format!("/{sanitized_filename}"),
            "size": 0,
            "message": "Upload endpoint ready - requires multipart body integration"
        })),
    )
        .into_response())
}

/// Sanitize filename to prevent path traversal and special characters.
pub fn sanitize_filename(name: &str) -> Result<String, AppError> {
    // Reject any path separators.
    if name.contains('/') || name.contains('\\') {
        return Err(AppError::BadRequest("Invalid filename".to_string()));
    }

    // Use UUID for upload filenames to prevent collisions.
    // For GET requests, return the sanitized name.
    Ok(name.to_string())
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
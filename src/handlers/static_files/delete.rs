use std::path::Path;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::{EndpointConfig, StaticFilesConfig};
use crate::error::AppError;
use crate::handlers::common::path::sanitize_filename;
use crate::storage::Storage;

/// Handle file deletion (DELETE).
///
/// # Errors
///
/// Returns an `AppError::MethodNotAllowed` if file management is not enabled.
pub async fn handle_file_delete(
    storage: &dyn Storage,
    endpoint: &EndpointConfig,
    config: &StaticFilesConfig,
    relative: &str,
    root: &Path,
) -> Result<Response, AppError> {
    let upload_config = config
        .upload
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed {
            message: "File management is not enabled".to_string(),
            allowed: endpoint.methods.clone(),
        })?;

    if !upload_config.enabled {
        return Err(AppError::MethodNotAllowed {
            message: "File management is not enabled".to_string(),
            allowed: endpoint.methods.clone(),
        });
    }

    let sanitized_filename = sanitize_filename(relative, false)?;

    let delete_path = root.join(&sanitized_filename);

    // Verify file exists.
    if !storage.exists(&delete_path).await {
        return Err(AppError::NotFound("File not found".to_string()));
    }

    // Delete the file.
    storage
        .delete(&delete_path)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to delete file: {e}")))?;

    Ok((
        StatusCode::NO_CONTENT,
        axum::Json(serde_json::json!({
            "success": true,
            "message": "File deleted successfully"
        })),
    )
        .into_response())
}

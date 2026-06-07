use std::path::Path;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::StaticFilesConfig;
use crate::error::AppError;
use crate::handlers::static_files::upload::sanitize_filename;
use crate::server::state::AppState;
use crate::storage::Storage;

/// Handle file deletion (DELETE).
pub async fn handle_file_delete(
    storage: &dyn Storage,
    _state: State<AppState>,
    _endpoint: &crate::config::types::EndpointConfig,
    config: &StaticFilesConfig,
    relative: &str,
    root: &Path,
    _headers: &axum::http::HeaderMap,
) -> Result<Response, AppError> {
    let upload_config = config
        .upload
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("File management is not enabled".to_string()))?;

    if !upload_config.enabled {
        return Err(AppError::MethodNotAllowed(
            "File management is not enabled".to_string(),
        ));
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

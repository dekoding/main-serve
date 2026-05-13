use std::collections::HashMap;
use std::path::Path;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::StaticFilesConfig;
use crate::error::AppError;
use crate::handlers::static_files::routing::{check_upload_role, extract_auth_info};
use crate::handlers::static_files::upload::sanitize_filename;
use crate::server::state::AppState;

/// Handle file deletion (DELETE).
pub async fn handle_file_delete(
    state: State<AppState>,
    endpoint: &crate::config::types::EndpointConfig,
    config: &StaticFilesConfig,
    relative: &str,
    root: &Path,
    headers: &axum::http::HeaderMap,
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

    let query_params: HashMap<String, String> = endpoint
        .crud
        .as_ref()
        .map(|c| {
            c.filtering
                .allowed_fields
                .iter()
                .cloned()
                .map(|s| (s.clone(), s))
                .collect()
        })
        .unwrap_or_default();
    let auth_info = extract_auth_info(&state, endpoint, headers, &query_params).await?;
    check_upload_role(&auth_info, upload_config)?;

    let user_id = &auth_info.subject;
    let sanitized_filename = sanitize_filename(relative, false)?;

    // Check user_scope.
    let delete_path = if let Some(user_scope) = &config.user_scope {
        if user_scope.enabled {
            let pattern = &user_scope.directory_pattern;
            let user_path =
                format!("{}/{user_id}/{sanitized_filename}", pattern).replace("//", "/");
            root.join(&user_path)
        } else {
            root.join(&sanitized_filename)
        }
    } else {
        root.join(&sanitized_filename)
    };

    // Verify file exists.
    if !delete_path.exists() {
        return Err(AppError::NotFound("File not found".to_string()));
    }

    // Delete the file.
    tokio::fs::remove_file(&delete_path)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to delete file: {e}")))?;

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": "File deleted successfully"
        })),
    )
        .into_response())
}

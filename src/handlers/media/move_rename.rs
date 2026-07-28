/// Media move and rename handlers.
use std::path::Path;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::{EndpointConfig, MediaConfig};
use crate::db::query::select_one::build_select_file_path;
use crate::db::query::update::build_set_file_path;
use crate::error::AppError;
use crate::handlers::common::helpers::extract_file_path;
use crate::handlers::common::path::validate_path_within;
use crate::storage::Storage;

/// Handle media move (POST /:id/move).
///
/// # Errors
///
/// Returns an `AppError::MethodNotAllowed` if move is not configured.
pub async fn handle_media_move(
    id: &str,
    endpoint: &EndpointConfig,
    config: &MediaConfig,
    pool: &crate::db::pool::DatabasePool,
    storage: &dyn Storage,
    root: &Path,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let move_config = config
        .move_config
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed {
            message: "Media move is not configured".to_string(),
            allowed: endpoint.methods.clone(),
        })?;

    if !move_config.enabled {
        return Err(AppError::MethodNotAllowed {
            message: "Media move is disabled".to_string(),
            allowed: endpoint.methods.clone(),
        });
    }

    let destination_path = body
        .get("destination_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("destination_path is required".to_string()))?;

    let built = build_select_file_path(&config.table, pool.driver());
    let row = pool.fetch_optional_json(&built.sql, &[id.into()]).await?;

    let current_file_path = extract_file_path(row, id)?;

    let current_path = root.join(&current_file_path);

    let new_path = if destination_path.starts_with('/') {
        root.join(destination_path.trim_start_matches('/'))
    } else {
        root.join(destination_path)
    };

    validate_path_within(&new_path, root)?;

    if move_config.auto_create_destination
        && let Some(parent) = new_path.parent()
    {
        storage
            .create_dir_all(parent)
            .await
            .map_err(|e| AppError::FileOperation(format!("Failed to create directory: {e}")))?;
    }

    storage
        .rename(&current_path, &new_path)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to move file: {e}")))?;

    let new_relative = new_path.strip_prefix(root).map_or_else(
        |_| format!("/{}", new_path.to_string_lossy()),
        |p| format!("/{}", p.to_string_lossy()),
    );

    let built = build_set_file_path(
        &config.table,
        new_relative.trim_start_matches('/'),
        pool.driver(),
    )
    .map_err(|e| AppError::Internal(format!("Failed to build query: {e}")))?;
    pool.execute_with_params(
        &built.sql,
        &[
            serde_json::Value::String(new_relative.trim_start_matches('/').to_string()),
            id.into(),
        ],
    )
    .await?;

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": "Media file moved successfully",
            "new_path": new_relative,
        })),
    )
        .into_response())
}

/// Handle media rename (PATCH /:id/rename).
///
/// # Errors
///
/// Returns an `AppError::MethodNotAllowed` if rename is not configured.
pub async fn handle_media_rename(
    id: &str,
    endpoint: &EndpointConfig,
    config: &MediaConfig,
    pool: &crate::db::pool::DatabasePool,
    storage: &dyn Storage,
    root: &Path,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let rename_config = config
        .rename
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed {
            message: "Media rename is not configured".to_string(),
            allowed: endpoint.methods.clone(),
        })?;

    if !rename_config.enabled {
        return Err(AppError::MethodNotAllowed {
            message: "Media rename is disabled".to_string(),
            allowed: endpoint.methods.clone(),
        });
    }

    let new_name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("name is required".to_string()))?;

    let built = build_select_file_path(&config.table, pool.driver());
    let row = pool.fetch_optional_json(&built.sql, &[id.into()]).await?;

    let current_file_path = extract_file_path(row, id)?;

    let current_path = root.join(&current_file_path);

    let parent = current_path
        .parent()
        .ok_or_else(|| AppError::BadRequest("Cannot determine parent directory".to_string()))?;

    let new_path = parent.join(new_name);

    validate_path_within(&new_path, root)?;

    storage
        .rename(&current_path, &new_path)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to rename file: {e}")))?;

    let new_relative = new_path.strip_prefix(root).map_or_else(
        |_| format!("/{}", new_path.to_string_lossy()),
        |p| format!("/{}", p.to_string_lossy()),
    );

    let built = build_set_file_path(
        &config.table,
        new_relative.trim_start_matches('/'),
        pool.driver(),
    )
    .map_err(|e| AppError::Internal(format!("Failed to build query: {e}")))?;
    pool.execute_with_params(
        &built.sql,
        &[
            serde_json::Value::String(new_relative.trim_start_matches('/').to_string()),
            id.into(),
        ],
    )
    .await?;

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": "Media file renamed successfully",
            "new_path": new_relative,
        })),
    )
        .into_response())
}

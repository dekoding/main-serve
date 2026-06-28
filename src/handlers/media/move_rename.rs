/// Media move and rename handlers.
use std::path::Path;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::MediaConfig;
use crate::db::migration::quote_object_name;
use crate::error::AppError;
use crate::storage::Storage;

/// Handle media move (POST /:id/move).
pub async fn handle_media_move(
    id: &str,
    config: &MediaConfig,
    pool: &crate::db::pool::DatabasePool,
    storage: &dyn Storage,
    root: &Path,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let move_config = config
        .move_config
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Media move is not configured".to_string()))?;

    if !move_config.enabled {
        return Err(AppError::MethodNotAllowed(
            "Media move is disabled".to_string(),
        ));
    }

    let destination_path = body
        .get("destination_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("destination_path is required".to_string()))?;

    let driver = pool.driver();
    let sql = format!(
        "SELECT file_path FROM {} WHERE id = $1",
        quote_object_name(&config.table, driver)
    );
    let row = pool.fetch_optional_json(&sql, &[id.into()]).await?;

    let current_file_path = row
        .and_then(|r| {
            r.get("file_path")
                .and_then(|v| v.as_str().map(String::from))
        })
        .ok_or_else(|| AppError::NotFound(format!("Media item with id '{}' not found", id)))?;

    let current_path = root.join(&current_file_path);

    let new_path = if destination_path.starts_with('/') {
        root.join(destination_path.trim_start_matches('/'))
    } else {
        root.join(destination_path)
    };

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

    let new_relative = new_path
        .strip_prefix(root)
        .map(|p| format!("/{}", p.to_string_lossy()))
        .unwrap_or_else(|_| format!("/{}", new_path.to_string_lossy()));

    let update_sql = format!(
        "UPDATE {} SET file_path = $1 WHERE id = $2",
        quote_object_name(&config.table, driver)
    );
    pool.execute_with_params(
        &update_sql,
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
pub async fn handle_media_rename(
    id: &str,
    config: &MediaConfig,
    pool: &crate::db::pool::DatabasePool,
    storage: &dyn Storage,
    root: &Path,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let rename_config = config
        .rename
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Media rename is not configured".to_string()))?;

    if !rename_config.enabled {
        return Err(AppError::MethodNotAllowed(
            "Media rename is disabled".to_string(),
        ));
    }

    let new_name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("name is required".to_string()))?;

    let driver = pool.driver();
    let sql = format!(
        "SELECT file_path FROM {} WHERE id = $1",
        quote_object_name(&config.table, driver)
    );
    let row = pool.fetch_optional_json(&sql, &[id.into()]).await?;

    let current_file_path = row
        .and_then(|r| {
            r.get("file_path")
                .and_then(|v| v.as_str().map(String::from))
        })
        .ok_or_else(|| AppError::NotFound(format!("Media item with id '{}' not found", id)))?;

    let current_path = root.join(&current_file_path);

    let parent = current_path
        .parent()
        .ok_or_else(|| AppError::BadRequest("Cannot determine parent directory".to_string()))?;

    let new_path = parent.join(new_name);

    storage
        .rename(&current_path, &new_path)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to rename file: {e}")))?;

    let new_relative = new_path
        .strip_prefix(root)
        .map(|p| format!("/{}", p.to_string_lossy()))
        .unwrap_or_else(|_| format!("/{}", new_path.to_string_lossy()));

    let update_sql = format!(
        "UPDATE {} SET file_path = $1 WHERE id = $2",
        quote_object_name(&config.table, driver)
    );
    pool.execute_with_params(
        &update_sql,
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

/// Media delete handlers.
use std::path::Path;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::MediaConfig;
use crate::db::query::builders::{build_delete, build_file_ref_select_by_file_id};
use crate::db::query::select_one::build_select_file_path;
use crate::error::AppError;
use crate::handlers::common::utils::{DatabaseContext, HandlerContext};
use crate::handlers::media::trash::handle_media_trash_delete;
use crate::middleware::auth::extractor::RequestContext;
use crate::storage::Storage;

/// Handle `DELETE` for a media endpoint.
///
/// Routes to trash or permanent delete based on configuration.
///
/// # Errors
///
/// Returns `AppError::Auth` on authentication failure. Returns `AppError::Internal`
/// or `AppError::FileOperation` on database or storage errors.
pub async fn handle_media_delete(
    handler_ctx: &HandlerContext<'_>,
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    db_ctx: &DatabaseContext,
) -> Result<Response, AppError> {
    let trash_enabled = config.trash.as_ref().is_some_and(|t| t.enabled);

    if trash_enabled {
        handle_media_trash_delete(handler_ctx, id, config, storage, root, db_ctx).await
    } else {
        delete_media_permanently(id, config, storage, root, db_ctx).await
    }
}

// collapsible_if suppressed: early return pattern would obscure the delete logic.
/// Permanently delete a media item by ID: removes the file from storage and the row from the database.
///
/// # Errors
///
/// Returns `AppError::NotFound` if the item does not exist. Returns `AppError::FileOperation`
/// if storage deletion fails. Returns `AppError::Internal` on database errors.
pub async fn delete_media_permanently(
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    db_ctx: &DatabaseContext,
) -> Result<Response, AppError> {
    // Check for content references if on_delete is Error.
    if let Some(content_refs) = &config.content_references
        && content_refs.on_delete == crate::config::types::MediaOnDeleteBehavior::Error
    {
        let built = build_file_ref_select_by_file_id(
            &content_refs.table,
            &content_refs.media_id_column,
            &content_refs.entity_id_column,
            &content_refs.content_type_column,
            db_ctx.pool.driver(),
        );
        let rows = db_ctx.pool.fetch_all_json(&built.sql, &[id.into()]).await?;

        if !rows.is_empty() {
            let count = rows.len();
            let details: Vec<String> = rows
                .iter()
                .map(|row| {
                    let entity_id = row
                        .get(&content_refs.entity_id_column)
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let content_type = row
                        .get(&content_refs.content_type_column)
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    format!("{content_type}:{entity_id}")
                })
                .collect();

            return Err(AppError::Conflict {
                message: format!(
                    "Media item '{id}' is attached to {count} content entity(ies). \
                     Use on_delete: detach or delete the references first.",
                ),
                details,
            });
        }
    }

    let driver = db_ctx.pool.driver();
    let built = build_select_file_path(&config.table, driver);
    let row = db_ctx
        .pool
        .fetch_optional_json(&built.sql, &[id.into()])
        .await?;

    if let Some(row) = row
        && let Some(file_path) = row.get("file_path").and_then(|v| v.as_str())
    {
        let file_path_buf = root.join(file_path);
        if storage.exists(&file_path_buf).await {
            storage
                .delete(&file_path_buf)
                .await
                .map_err(|e| AppError::FileOperation(format!("Failed to delete file: {e}")))?;
        }
    }

    let built = build_delete(id, db_ctx, &RequestContext::default(), &None)?;
    let rows_affected = db_ctx
        .pool
        .execute_with_params(&built.sql, &built.params)
        .await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "Media item with id '{id}' not found"
        )));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
    )
        .into_response())
}

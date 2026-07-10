/// Media delete handlers.
use std::path::Path;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::MediaConfig;
use crate::db::query::builders::build_delete;
use crate::db::query::select_one::build_select_file_path;
use crate::error::AppError;
use crate::handlers::common::utils::{DatabaseContext, HandlerContext};
use crate::handlers::media::trash::handle_media_trash_delete;
use crate::middleware::auth::extractor::RequestContext;
use crate::storage::Storage;

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
pub async fn delete_media_permanently(
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    db_ctx: &DatabaseContext,
) -> Result<Response, AppError> {
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
            "Media item with id '{}' not found",
            &id
        )));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
    )
        .into_response())
}

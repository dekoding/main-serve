use std::path::PathBuf;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::db::query::builders::build_delete;
use crate::db::query::select_one::build_select_file_path;
use crate::db::query::update::build_set_trashed;
use crate::error::AppError;
use crate::handlers::file_store::{FileStoreContext, check_file_store_ownership};
use crate::middleware::auth::extractor::RequestContext;

/// Handle deleting a file store entry.
///
/// # Errors
///
/// Returns an error on authentication failure, ownership violations, or
/// database and storage errors.
pub async fn handle_file_store_delete(ctx: &FileStoreContext<'_>) -> Result<Response, AppError> {
    let pool = &ctx.db_ctx.pool;
    let driver = &ctx.db_ctx.pool.driver();
    let storage = ctx.require_storage()?;
    let id = ctx.require_id()?;
    let trash_enabled = ctx.config.trash.as_ref().is_some_and(|t| t.enabled);

    let check_ownership = ctx.config.ownership.is_some();
    if check_ownership {
        let auth_info = ctx.handler_ctx.extract_auth_info().await?;
        if !ctx.handler_ctx.endpoint.roles.is_admin(&auth_info.role) {
            check_file_store_ownership(ctx.handler_ctx, ctx.config, id, driver).await?;
        }
    }

    if trash_enabled {
        let built = build_select_file_path(&ctx.config.table, ctx.db_ctx.pool.driver());
        let row = pool.fetch_optional_json(&built.sql, &[id.into()]).await?;
        if let Some(row) = row
            && let Some(file_path) = row.get("file_path").and_then(|v| v.as_str())
        {
            let store_root = storage
                .root_path()
                .unwrap_or_else(|| PathBuf::from(&ctx.config.storage));
            let trash_dest = store_root.join(".trash").join(file_path);
            let source_path = store_root.join(file_path);

            if storage.exists(&source_path).await {
                if let Some(parent) = trash_dest.parent() {
                    storage.create_dir_all(parent).await.map_err(|e| {
                        AppError::FileOperation(format!("Failed to create trash directory: {e}"))
                    })?;
                }
                let _ = storage.rename(&source_path, &trash_dest).await;
            }
        }

        let built = build_set_trashed(&ctx.config.table, ctx.db_ctx.pool.driver())
            .map_err(|e| AppError::Internal(format!("Failed to build query: {e}")))?;
        let rows_affected = pool.execute_with_params(&built.sql, &[id.into()]).await?;

        if rows_affected == 0 {
            return Err(AppError::NotFound(format!(
                "File entry with id '{id}' not found"
            )));
        }

        Ok((StatusCode::NO_CONTENT,).into_response())
    } else {
        if ctx.config.ownership.is_some() {
            let auth_info = ctx.handler_ctx.extract_auth_info().await?;
            if !ctx.handler_ctx.endpoint.roles.is_admin(&auth_info.role) {
                check_file_store_ownership(ctx.handler_ctx, ctx.config, id, driver).await?;
            }
        }

        let built = build_select_file_path(&ctx.config.table, ctx.db_ctx.pool.driver());
        let row = pool.fetch_optional_json(&built.sql, &[id.into()]).await?;
        if let Some(row) = row
            && let Some(file_path) = row.get("file_path").and_then(|v| v.as_str())
        {
            let file_path_buf = storage
                .root_path()
                .unwrap_or_else(|| PathBuf::from(&ctx.config.storage))
                .join(file_path);
            if storage.exists(&file_path_buf).await {
                storage
                    .delete(&file_path_buf)
                    .await
                    .map_err(|e| AppError::FileOperation(format!("Failed to delete file: {e}")))?;
            }
        }

        let built = build_delete(id, ctx.db_ctx, &RequestContext::default(), &None)?;
        let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;

        if rows_affected == 0 {
            return Err(AppError::NotFound(format!(
                "File entry with id '{id}' not found"
            )));
        }

        Ok((StatusCode::NO_CONTENT,).into_response())
    }
}

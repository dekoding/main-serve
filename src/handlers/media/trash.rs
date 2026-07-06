/// Media trash management handlers.
use std::path::Path;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::MediaConfig;
use crate::db::query::builders::build_delete;
use crate::db::query::select_one::{
    build_select_file_path, build_select_trashed, build_select_trashed_ids,
    build_select_trashed_item,
};
use crate::db::query::update::{build_set_restored, build_set_trashed};
use crate::error::AppError;
use crate::handlers::common::helpers::extract_file_path;
use crate::handlers::common::utils::{DatabaseContext, HandlerContext};
use crate::middleware::auth::extractor::RequestContext;
use crate::storage::Storage;

/// Handle trash management routes.
pub async fn handle_media_trash(
    handler_ctx: &HandlerContext<'_>,
    method: axum::http::Method,
    path: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    db_ctx: &DatabaseContext,
) -> Result<Response, AppError> {
    let trash_config = config
        .trash
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Trash is not enabled".to_string()))?;

    if !trash_config.enabled {
        return Err(AppError::MethodNotAllowed(
            "Trash is not enabled".to_string(),
        ));
    }

    match method {
        axum::http::Method::GET => handle_media_trash_list(db_ctx).await,
        axum::http::Method::DELETE
            if path == "/_main-serve/media/trash" || path == "/_main-serve/media/trash/" =>
        {
            handle_media_trash_empty(config, db_ctx).await
        }
        axum::http::Method::POST => {
            if let Some(id) = path
                .strip_prefix("/_main-serve/media/trash/")
                .and_then(|p| p.strip_suffix("/restore"))
            {
                handle_media_trash_restore(handler_ctx, id, config, storage, root, db_ctx).await
            } else {
                Err(AppError::BadRequest(
                    "Invalid trash restore path".to_string(),
                ))
            }
        }
        axum::http::Method::DELETE => {
            if let Some(id) = path.strip_prefix("/_main-serve/media/trash/") {
                handle_media_trash_permanent_delete(handler_ctx, id, config, storage, root, db_ctx)
                    .await
            } else {
                Err(AppError::BadRequest(
                    "Invalid trash delete path".to_string(),
                ))
            }
        }
        _ => Err(AppError::MethodNotAllowed(
            "Method not allowed for trash endpoint".to_string(),
        )),
    }
}

pub async fn handle_media_trash_list(db_ctx: &DatabaseContext) -> Result<Response, AppError> {
    let built = build_select_trashed(&db_ctx.table_config.name, db_ctx.pool.driver());
    let rows = db_ctx.pool.fetch_all_json(&built.sql, &[]).await?;

    Ok((StatusCode::OK, axum::Json(rows)).into_response())
}

pub async fn handle_media_trash_restore(
    handler_ctx: &HandlerContext<'_>,
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    db_ctx: &DatabaseContext,
) -> Result<Response, AppError> {
    let trash_config = config
        .trash
        .as_ref()
        .ok_or_else(|| AppError::Internal("Trash not enabled".to_string()))?;

    let built = build_select_trashed_item(&config.table, db_ctx.pool.driver());
    let row = db_ctx
        .pool
        .fetch_optional_json(&built.sql, &[id.into()])
        .await?;

    let file_path = extract_file_path(row, id).await?;
    let auth_info = handler_ctx.extract_auth_info().await?;
    let user_path = auth_info.subject;

    let trash_path = root
        .join(&trash_config.prefix)
        .join(&user_path)
        .join(&file_path);
    let restore_path = root.join(file_path);

    if storage.exists(&trash_path).await {
        if let Some(parent) = restore_path.parent() {
            storage
                .create_dir_all(parent)
                .await
                .map_err(|e| AppError::FileOperation(format!("Failed to create directory: {e}")))?;
        }
        storage
            .rename(&trash_path, &restore_path)
            .await
            .map_err(|e| AppError::FileOperation(format!("Failed to restore file: {e}")))?;
    }

    let built = build_set_restored(&config.table, db_ctx.pool.driver())
        .map_err(|e| AppError::Internal(format!("Failed to build query: {e}")))?;
    db_ctx
        .pool
        .execute_with_params(&built.sql, &[id.into()])
        .await?;

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": "Media item restored from trash"
        })),
    )
        .into_response())
}

// These functions need access to configs and database pool.
#[allow(clippy::too_many_arguments)]
pub async fn handle_media_trash_empty(
    config: &MediaConfig,
    db_ctx: &DatabaseContext,
) -> Result<Response, AppError> {
    let built = build_select_trashed_ids(&config.table, db_ctx.pool.driver());
    let rows = db_ctx.pool.fetch_all_json(&built.sql, &[]).await?;

    let mut deleted_count = 0u64;

    for row in &rows {
        if let Some(id_val) = row.get("id").and_then(|v| v.as_str()) {
            let built = build_delete(id_val, db_ctx, &RequestContext::default(), &None)?;
            let _ = db_ctx
                .pool
                .execute_with_params(&built.sql, &built.params)
                .await;
            deleted_count += 1;
        }
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": format!("Emptied trash, deleted {} items", deleted_count),
            "deleted_count": deleted_count,
        })),
    )
        .into_response())
}

pub async fn handle_media_trash_permanent_delete(
    handler_ctx: &HandlerContext<'_>,
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    db_ctx: &DatabaseContext,
) -> Result<Response, AppError> {
    let trash_config = config
        .trash
        .as_ref()
        .ok_or_else(|| AppError::Internal("Trash not enabled".to_string()))?;

    let built = build_select_trashed_item(&config.table, db_ctx.pool.driver());
    let row = db_ctx
        .pool
        .fetch_optional_json(&built.sql, &[id.into()])
        .await?;

    let file_path = extract_file_path(row, id).await?;

    if !file_path.is_empty() {
        let auth_info = handler_ctx.extract_auth_info().await?;
        let trash_path = root
            .join(&trash_config.prefix)
            .join(&auth_info.subject)
            .join(&file_path);
        if storage.exists(&trash_path).await {
            let _ = storage.delete(&trash_path).await;
        }
    }

    let built = build_delete(id, db_ctx, &RequestContext::default(), &None)?;
    let rows_affected = db_ctx
        .pool
        .execute_with_params(&built.sql, &built.params)
        .await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(
            "Trashed media item not found".to_string(),
        ));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": "Media item permanently deleted from trash",
            "rows_affected": rows_affected,
        })),
    )
        .into_response())
}

pub async fn handle_media_trash_delete(
    handler_ctx: &HandlerContext<'_>,
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    db_ctx: &DatabaseContext,
) -> Result<Response, AppError> {
    let trash_config = config
        .trash
        .as_ref()
        .ok_or_else(|| AppError::Internal("Trash not enabled but delete was called".to_string()))?;

    let auth_info = handler_ctx.extract_auth_info().await?;
    let user_id = auth_info.subject;

    let built = build_select_file_path(&config.table, db_ctx.pool.driver());
    let row = db_ctx
        .pool
        .fetch_optional_json(&built.sql, &[id.into()])
        .await?;

    let file_path = extract_file_path(row, id).await?;

    let user_path = user_id.clone();

    let trash_prefix = &trash_config.prefix;
    let trash_dest = root.join(trash_prefix).join(&user_path).join(&file_path);

    if let Some(parent) = trash_dest.parent() {
        storage.create_dir_all(parent).await.map_err(|e| {
            AppError::FileOperation(format!("Failed to create trash directory: {e}"))
        })?;
    }

    let source_path = root.join(file_path);
    if storage.exists(&source_path).await {
        storage
            .rename(&source_path, &trash_dest)
            .await
            .map_err(|e| AppError::FileOperation(format!("Failed to move file to trash: {e}")))?;
    }

    let built = build_set_trashed(&config.table, db_ctx.pool.driver())
        .map_err(|e| AppError::Internal(format!("Failed to build query: {e}")))?;
    let rows_affected = db_ctx
        .pool
        .execute_with_params(&built.sql, &[id.into()])
        .await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "Media item with id '{}' not found",
            &id
        )));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": "Media moved to trash",
            "rows_affected": rows_affected,
        })),
    )
        .into_response())
}

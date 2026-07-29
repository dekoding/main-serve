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

/// Handle trash management routes for media.
///
/// Dispatches to list, empty, restore, or permanent-delete handlers
/// based on the HTTP method and path.
///
/// # Errors
///
/// Returns `AppError::MethodNotAllowed` if trash is not enabled.
/// Returns `AppError::BadRequest` on invalid paths.
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
        .ok_or_else(|| AppError::MethodNotAllowed {
            message: "Trash is not enabled".to_string(),
            allowed: handler_ctx.endpoint.methods.clone(),
        })?;

    if !trash_config.enabled {
        return Err(AppError::MethodNotAllowed {
            message: "Trash is not enabled".to_string(),
            allowed: handler_ctx.endpoint.methods.clone(),
        });
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
        _ => Err(AppError::MethodNotAllowed {
            message: "Method not allowed for trash endpoint".to_string(),
            allowed: handler_ctx.endpoint.methods.clone(),
        }),
    }
}

/// List all trashed media items.
///
/// # Errors
///
/// Returns `AppError::Internal` on database errors.
pub async fn handle_media_trash_list(db_ctx: &DatabaseContext) -> Result<Response, AppError> {
    let built = build_select_trashed(&db_ctx.table_config.name, db_ctx.pool.driver());
    let rows = db_ctx.pool.fetch_all_json(&built.sql, &[]).await?;

    Ok((StatusCode::OK, axum::Json(rows)).into_response())
}

/// Restore a trashed media item by moving it back to its original location.
///
/// The file is renamed from the trash directory back to its original path,
/// and the database row's trash flag is cleared.
///
/// # Errors
///
/// Returns `AppError::Internal` if trash is not enabled. Returns `AppError::Auth`
/// on authentication failure. Returns `AppError::FileOperation` on storage errors.
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

    let file_path = extract_file_path(row, id)?;
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

/// Permanently delete all trashed media items (empty the trash).
///
/// Deletes every database row currently in the trash. Individual files in
/// storage are left behind (they will be cleaned up by the retention policy).
///
/// # Errors
///
/// Returns `AppError::Internal` on database errors.
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

/// Permanently delete a single trashed media item.
///
/// Deletes the file from the trash directory and removes the database row.
///
/// # Errors
///
/// Returns `AppError::Internal` if trash is not enabled. Returns `AppError::NotFound`
/// if the trashed item does not exist. Returns `AppError::FileOperation` on storage errors.
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

    let file_path = extract_file_path(row, id)?;

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

/// Move a media item to trash (soft delete).
///
/// Renames the file from its original location to the trash directory under
/// the user's path, and updates the database row's trash flag.
///
/// # Errors
///
/// Returns `AppError::Internal` if trash is not enabled. Returns `AppError::Auth`
/// on authentication failure. Returns `AppError::FileOperation` on storage errors.
/// Returns `AppError::NotFound` if the media item does not exist.
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

    let file_path = extract_file_path(row, id)?;

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
            "Media item with id '{id}' not found"
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

use std::path::Path;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::FileStoreConfig;
use crate::db::query::builders::build_delete;
use crate::db::query::select_one::{
    build_select_file_path, build_select_trashed, build_select_trashed_ids,
    build_select_trashed_item,
};
use crate::db::query::update::build_set_restored;
use crate::error::AppError;
use crate::handlers::common::helpers::extract_file_path;
use crate::handlers::common::utils::DatabaseContext;
use crate::middleware::auth::extractor::RequestContext;
use crate::storage::Storage;

/// Handle file store trash management routes.
pub async fn handle_file_store_trash(
    method: axum::http::Method,
    path: &str,
    config: &FileStoreConfig,
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
        axum::http::Method::GET => handle_file_store_trash_list(db_ctx).await,
        axum::http::Method::DELETE
            if path == "/_main-serve/file-store/trash"
                || path == "/_main-serve/file-store/trash/" =>
        {
            handle_file_store_trash_empty(config, db_ctx, storage, root).await
        }
        axum::http::Method::POST => {
            if let Some(id) = path
                .strip_prefix("/_main-serve/file-store/trash/")
                .and_then(|p| p.strip_suffix("/restore"))
            {
                handle_file_store_trash_restore(id, config, storage, root, db_ctx).await
            } else {
                Err(AppError::BadRequest(
                    "Invalid trash restore path".to_string(),
                ))
            }
        }
        axum::http::Method::DELETE => {
            if let Some(id) = path.strip_prefix("/_main-serve/file-store/trash/") {
                handle_file_store_trash_permanent_delete(id, config, storage, root, db_ctx).await
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

async fn handle_file_store_trash_list(db_ctx: &DatabaseContext) -> Result<Response, AppError> {
    let built = build_select_trashed(&db_ctx.table_config.name, db_ctx.pool.driver());
    let rows = db_ctx.pool.fetch_all_json(&built.sql, &[]).await?;
    let total: i64 = rows.len() as i64;

    let response = serde_json::json!({
        "data": rows,
        "pagination": {
            "page": 1,
            "page_size": total,
            "total": total,
            "total_pages": 1,
        }
    });

    Ok((StatusCode::OK, axum::Json(response)).into_response())
}

async fn handle_file_store_trash_restore(
    id: &str,
    config: &FileStoreConfig,
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

    let file_path = extract_file_path(row, id).unwrap_or_default();

    let trash_path = root.join(&trash_config.prefix).join(file_path.clone());
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
            "message": "File store entry restored from trash"
        })),
    )
        .into_response())
}

async fn handle_file_store_trash_empty(
    config: &FileStoreConfig,
    db_ctx: &DatabaseContext,
    storage: &dyn Storage,
    root: &Path,
) -> Result<Response, AppError> {
    let trash_config = config
        .trash
        .as_ref()
        .ok_or_else(|| AppError::Internal("Trash not enabled".to_string()))?;

    let built = build_select_trashed_ids(&config.table, db_ctx.pool.driver());
    let mut rows = db_ctx.pool.fetch_all_json(&built.sql, &[]).await?;

    // Convert numeric ids to strings for consistency
    for row in &mut rows {
        if let serde_json::Value::Object(obj) = row
            && let Some(id_val) = obj.get("id")
        {
            let id_str = id_val
                .as_i64()
                .map(|i| i.to_string())
                .unwrap_or_else(|| id_val.to_string());
            obj.insert("id".to_string(), serde_json::Value::String(id_str));
        }
    }

    let mut deleted_count = 0u64;

    for row in &rows {
        if let Some(id_val) = row.get("id").and_then(|v| v.as_str()) {
            // Try to delete the file from trash storage
            let path_built = build_select_file_path(&config.table, db_ctx.pool.driver());
            let path_row = db_ctx
                .pool
                .fetch_optional_json(&path_built.sql, &[id_val.into()])
                .await?;

            if let Some(path_row) = path_row
                && let Some(fp) = path_row.get("file_path").and_then(|v| v.as_str())
            {
                let trash_path = root.join(&trash_config.prefix).join(fp);
                if storage.exists(&trash_path).await {
                    let _ = storage.delete(&trash_path).await;
                }
            }

            // Delete the DB record
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

async fn handle_file_store_trash_permanent_delete(
    id: &str,
    config: &FileStoreConfig,
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

    let file_path = extract_file_path(row, id).unwrap_or_default();

    if !file_path.is_empty() {
        let trash_path = root.join(&trash_config.prefix).join(&file_path);
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
            "Trashed file store entry not found".to_string(),
        ));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": "File store entry permanently deleted from trash",
            "rows_affected": rows_affected,
        })),
    )
        .into_response())
}

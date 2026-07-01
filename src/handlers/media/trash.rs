/// Media trash management handlers.
use std::collections::HashMap;
use std::path::Path;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::{EndpointConfig, MediaConfig, TableConfig};
use crate::db::query::builders::build_delete;
use crate::db::query::select_one::{
    build_select_file_path, build_select_trashed, build_select_trashed_ids,
    build_select_trashed_item,
};
use crate::db::query::update::{build_set_restored, build_set_trashed};
use crate::error::AppError;
use crate::handlers::common::extract_auth_info;
use crate::middleware::auth::extractor::RequestContext;
use crate::storage::Storage;

/// Handle trash management routes.
// These functions need access to state, configs, headers, query params, and storage.
#[allow(clippy::too_many_arguments)]
pub async fn handle_media_trash(
    state: &crate::server::state::AppState,
    method: axum::http::Method,
    path: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    pool: &crate::db::pool::DatabasePool,
    table_config: &TableConfig,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
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
        axum::http::Method::GET => handle_media_trash_list(pool, table_config).await,
        axum::http::Method::DELETE
            if path == "/_main-serve/media/trash" || path == "/_main-serve/media/trash/" =>
        {
            handle_media_trash_empty(config, pool, table_config).await
        }
        axum::http::Method::POST => {
            if let Some(id) = path
                .strip_prefix("/_main-serve/media/trash/")
                .and_then(|p| p.strip_suffix("/restore"))
            {
                handle_media_trash_restore(
                    state,
                    id,
                    config,
                    storage,
                    root,
                    pool,
                    endpoint,
                    headers,
                    query_params,
                )
                .await
            } else {
                Err(AppError::BadRequest(
                    "Invalid trash restore path".to_string(),
                ))
            }
        }
        axum::http::Method::DELETE => {
            if let Some(id) = path.strip_prefix("/_main-serve/media/trash/") {
                handle_media_trash_permanent_delete(
                    state,
                    id,
                    config,
                    storage,
                    root,
                    pool,
                    table_config,
                    endpoint,
                    headers,
                    query_params,
                )
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

pub async fn handle_media_trash_list(
    pool: &crate::db::pool::DatabasePool,
    table_config: &TableConfig,
) -> Result<Response, AppError> {
    let built = build_select_trashed(&table_config.name, pool.driver());
    let rows = pool.fetch_all_json(&built.sql, &[]).await?;

    Ok((StatusCode::OK, axum::Json(rows)).into_response())
}

// These functions need access to state, configs, headers, query params, and storage.
#[allow(clippy::too_many_arguments)]
pub async fn handle_media_trash_restore(
    state: &crate::server::state::AppState,
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    pool: &crate::db::pool::DatabasePool,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    let trash_config = config
        .trash
        .as_ref()
        .ok_or_else(|| AppError::Internal("Trash not enabled".to_string()))?;

    let built = build_select_trashed_item(&config.table, pool.driver());
    let row = pool.fetch_optional_json(&built.sql, &[id.into()]).await?;

    let file_path = match row {
        Some(r) => r
            .get("file_path")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default(),
        None => {
            return Err(AppError::NotFound(
                "Trashed media item not found".to_string(),
            ));
        }
    };

    let auth_info = extract_auth_info(state, endpoint, headers, query_params).await?;
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

    let built = build_set_restored(&config.table, pool.driver())
        .map_err(|e| AppError::Internal(format!("Failed to build query: {e}")))?;
    pool.execute_with_params(&built.sql, &[id.into()]).await?;

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
    pool: &crate::db::pool::DatabasePool,
    table_config: &TableConfig,
) -> Result<Response, AppError> {
    let built = build_select_trashed_ids(&config.table, pool.driver());
    let rows = pool.fetch_all_json(&built.sql, &[]).await?;

    let mut deleted_count = 0u64;

    for row in &rows {
        if let Some(id_val) = row.get("id").and_then(|v| v.as_str()) {
            let built = build_delete(
                table_config,
                id_val,
                pool.driver(),
                &RequestContext::default(),
                &None,
            )?;
            let _ = pool.execute_with_params(&built.sql, &built.params).await;
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

// These functions need access to state, configs, headers, query params, and storage.
#[allow(clippy::too_many_arguments)]
pub async fn handle_media_trash_permanent_delete(
    state: &crate::server::state::AppState,
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    pool: &crate::db::pool::DatabasePool,
    table_config: &TableConfig,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    let trash_config = config
        .trash
        .as_ref()
        .ok_or_else(|| AppError::Internal("Trash not enabled".to_string()))?;

    let built = build_select_trashed_item(&config.table, pool.driver());
    let row = pool.fetch_optional_json(&built.sql, &[id.into()]).await?;

    let file_path: String = if let Some(r) = row {
        r.get("file_path")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default()
    } else {
        String::new()
    };

    if !file_path.is_empty() {
        let auth_info = extract_auth_info(state, endpoint, headers, query_params).await?;
        let trash_path = root
            .join(&trash_config.prefix)
            .join(&auth_info.subject)
            .join(&file_path);
        if storage.exists(&trash_path).await {
            let _ = storage.delete(&trash_path).await;
        }
    }

    let built = build_delete(
        table_config,
        id,
        pool.driver(),
        &RequestContext::default(),
        &None,
    )?;
    let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;

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

// These functions need access to state, configs, headers, query params, and storage.
#[allow(clippy::too_many_arguments)]
pub async fn handle_media_trash_delete(
    state: &crate::server::state::AppState,
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    pool: &crate::db::pool::DatabasePool,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    let trash_config = config
        .trash
        .as_ref()
        .ok_or_else(|| AppError::Internal("Trash not enabled but delete was called".to_string()))?;

    let auth_info = extract_auth_info(state, endpoint, headers, query_params).await?;
    let user_id = auth_info.subject;

    let built = build_select_file_path(&config.table, pool.driver());
    let row = pool.fetch_optional_json(&built.sql, &[id.into()]).await?;

    let file_path = row
        .as_ref()
        .and_then(|r| {
            r.get("file_path")
                .and_then(|v| v.as_str().map(String::from))
        })
        .unwrap_or_default();

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

    let built = build_set_trashed(&config.table, pool.driver())
        .map_err(|e| AppError::Internal(format!("Failed to build query: {e}")))?;
    let rows_affected = pool.execute_with_params(&built.sql, &[id.into()]).await?;

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

/// File store (database-backed file catalog) handler.
use std::collections::HashMap;
use std::path::PathBuf;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::{DatabaseDriver, EndpointConfig, FileStoreConfig};
use crate::db::query::builders::{
    build_delete, build_insert, build_select_list, build_select_one, build_update,
};
use crate::db::query::helpers::{build_select_list_count, extract_query_params};
use crate::error::AppError;
use crate::middleware::auth::extractor::RequestContext;
use crate::server::state::AppState;
use crate::storage::Storage;

/// Route handler for file store endpoints.
pub async fn handle_file_store_route(
    state: axum::extract::State<AppState>,
    matched_path: axum::extract::MatchedPath,
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
    query: axum::extract::Query<HashMap<String, String>>,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config_for_method(path_str, &method)
        .await
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    let body_value = if body.is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
    };

    handle_file_store(
        &state,
        method,
        uri,
        &endpoint,
        headers,
        query.0,
        &body_value,
    )
    .await
}

/// Core file store handler logic.
pub async fn handle_file_store(
    state: &AppState,
    method: axum::http::Method,
    uri: axum::http::Uri,
    endpoint: &EndpointConfig,
    headers: axum::http::HeaderMap,
    query_params: HashMap<String, String>,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let config = endpoint
        .file_store
        .as_ref()
        .ok_or_else(|| AppError::NotFound("File store config not found".to_string()))?;

    let path = uri.path().to_string();

    let storage = state
        .get_store(&config.storage)
        .ok_or_else(|| AppError::Internal(format!("Store '{}' not found", config.storage)))?;

    let (pool, table_config, driver) = get_db_context(state, config).await?;

    match method {
        axum::http::Method::GET => {
            if path.is_empty() || path == "/" || path == config.table {
                handle_file_store_list(
                    state,
                    &pool,
                    config,
                    &table_config,
                    driver,
                    &query_params,
                    endpoint,
                    &headers,
                )
                .await
            } else {
                match extract_file_id(&path) {
                    Some(id) => {
                        handle_file_store_get(&pool, config, &table_config, driver, &id).await
                    }
                    None => Err(AppError::BadRequest("File ID required".to_string())),
                }
            }
        }
        axum::http::Method::POST => {
            if path.is_empty() || path == "/" {
                handle_file_store_create(
                    &pool,
                    config,
                    &table_config,
                    driver,
                    endpoint,
                    &headers,
                    body,
                    state,
                    &query_params,
                )
                .await
            } else {
                Err(AppError::MethodNotAllowed(
                    "POST not allowed on this path".to_string(),
                ))
            }
        }
        axum::http::Method::PATCH => match extract_file_id(&path) {
            Some(id) => {
                handle_file_store_update(
                    state,
                    &id,
                    config,
                    &table_config,
                    driver,
                    endpoint,
                    &headers,
                    body,
                    &query_params,
                )
                .await
            }
            None => Err(AppError::BadRequest("File ID required".to_string())),
        },
        axum::http::Method::DELETE => match extract_file_id(&path) {
            Some(id) => {
                handle_file_store_delete(
                    state,
                    &id,
                    config,
                    &*storage,
                    endpoint,
                    &pool,
                    &table_config,
                    driver,
                    &headers,
                    &query_params,
                )
                .await
            }
            None => Err(AppError::BadRequest("File ID required".to_string())),
        },
        _ => Err(AppError::MethodNotAllowed(
            "Method not allowed for file store endpoint".to_string(),
        )),
    }
}

// These functions need access to AppState, pool, configs, headers, and query params.
#[allow(clippy::too_many_arguments)]
async fn handle_file_store_list(
    state: &AppState,
    pool: &crate::db::pool::DatabasePool,
    config: &FileStoreConfig,
    table_config: &crate::config::types::TableConfig,
    driver: DatabaseDriver,
    query_params: &HashMap<String, String>,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
) -> Result<Response, AppError> {
    let mut qp = extract_query_params(query_params);

    // Apply default pagination from config when not specified by client.
    qp.page.get_or_insert(config.pagination.default_page_size);
    qp.page_size
        .get_or_insert(config.pagination.default_page_size);

    // Apply default sorting from config when not specified by client.
    if qp.sort.is_none() && !config.sorting.default_field.is_empty() {
        qp.sort = Some(config.sorting.default_field.clone());
    }
    if qp.order.is_none() {
        qp.order = Some(config.sorting.default_order);
    }

    // Apply ownership filter for non-admin users.
    if config.ownership.as_ref().is_some_and(|o| !o.admin_override) {
        qp.filters.insert(
            "owner_id".to_string(),
            extract_user_id(state, endpoint, headers, query_params).await?,
        );
    }

    let built = build_select_list(
        &config.table,
        table_config,
        &crate::config::types::CrudConfig::default(),
        &qp,
        driver,
        &RequestContext::default(),
    )?;
    let rows = pool.fetch_all_json(&built.sql, &built.params).await?;

    let count_built = build_select_list_count(&config.table, driver, &qp)?;
    let count_row = pool
        .fetch_optional_json(&count_built.sql, &count_built.params)
        .await?;
    let total: i64 = count_row
        .as_ref()
        .and_then(|r| r.get("count"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    // Apply field permissions to response rows
    let rows = apply_row_permissions(&rows, config).await?;

    let page = qp.page.unwrap_or(config.pagination.default_page_size);
    let page_size = qp.page_size.unwrap_or(config.pagination.default_page_size);

    let response = serde_json::json!({
        "data": rows,
        "pagination": {
            "page": page,
            "page_size": page_size,
            "total": total,
            "total_pages": (total as f64 / page_size as f64).ceil() as u64,
        }
    });

    Ok((StatusCode::OK, axum::Json(response)).into_response())
}

async fn handle_file_store_get(
    pool: &crate::db::pool::DatabasePool,
    config: &FileStoreConfig,
    table_config: &crate::config::types::TableConfig,
    driver: DatabaseDriver,
    id: &str,
) -> Result<Response, AppError> {
    let built = build_select_one(
        &config.table,
        table_config,
        &crate::config::types::CrudConfig::default(),
        id,
        driver,
        &RequestContext::default(),
    )?;
    match pool.fetch_optional_json(&built.sql, &built.params).await? {
        Some(mut row) => {
            if let Some(permissions) = &config.field_permissions {
                let mut filtered = serde_json::Map::new();
                if let Some(obj) = row.as_object_mut() {
                    let keys: Vec<String> = obj.keys().cloned().collect();
                    for key in keys {
                        if let Some(value) = obj.remove(&key)
                            && is_field_readable(&key, permissions).await
                        {
                            filtered.insert(key, value);
                        }
                    }
                }
                row = serde_json::Value::Object(filtered);
            }
            Ok((StatusCode::OK, axum::Json(row)).into_response())
        }
        None => Err(AppError::NotFound(format!(
            "File entry with id '{}' not found",
            id
        ))),
    }
}

// These functions need access to pool, configs, headers, query params, and body.
#[allow(clippy::too_many_arguments)]
async fn handle_file_store_create(
    pool: &crate::db::pool::DatabasePool,
    config: &FileStoreConfig,
    table_config: &crate::config::types::TableConfig,
    driver: DatabaseDriver,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    body: &serde_json::Value,
    state: &AppState,
    query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    let user_id = extract_user_id(state, endpoint, headers, query_params).await?;

    let writable_columns = table_config
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect::<Vec<_>>();
    let mut body_map = serde_json::Map::new();

    if let Some(obj) = body.as_object() {
        for (k, v) in obj {
            if writable_columns.contains(&k.to_string()) {
                body_map.insert(k.clone(), v.clone());
            }
        }
    }

    if config.ownership.is_some() {
        body_map.insert(
            "owner_id".to_string(),
            serde_json::Value::String(user_id.clone()),
        );
    }

    if writable_columns.contains(&"created_at".to_string()) {
        body_map.insert(
            "created_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
    }

    if writable_columns.contains(&"updated_at".to_string()) {
        body_map.insert(
            "updated_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
    }

    let json_body = serde_json::Value::Object(body_map);
    let built = build_insert(
        &config.table,
        table_config,
        &crate::config::types::CrudConfig::default(),
        &json_body,
        driver,
        &RequestContext::default(),
    )?;

    if built.sql.contains("RETURNING") {
        let row = pool.fetch_optional_json(&built.sql, &built.params).await?;
        Ok((
            StatusCode::CREATED,
            axum::Json(serde_json::json!({ "data": row })),
        )
            .into_response())
    } else {
        let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;
        Ok((
            StatusCode::CREATED,
            axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
        )
            .into_response())
    }
}

// These functions need access to state, configs, headers, and query params for ownership checks.
#[allow(clippy::too_many_arguments)]
async fn handle_file_store_update(
    state: &AppState,
    id: &str,
    config: &FileStoreConfig,
    table_config: &crate::config::types::TableConfig,
    driver: DatabaseDriver,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    body: &serde_json::Value,
    query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    // Check ownership for non-admin users before update.
    if config.ownership.as_ref().is_some_and(|o| !o.admin_override) {
        check_file_store_ownership(state, config, id, endpoint, headers, query_params).await?;
    }

    let pool = {
        let pools = state.db_pools.read().await;
        pools
            .get(&config.database)
            .ok_or_else(|| {
                AppError::Internal(format!("Database '{}' has no pool", config.database))
            })?
            .clone()
    };

    let mut body_map = serde_json::Map::new();
    if let Some(obj) = body.as_object() {
        for (k, v) in obj {
            if table_config.columns.iter().any(|c| c.name == *k) {
                body_map.insert(k.clone(), v.clone());
            }
        }
    }
    body_map.insert(
        "updated_at".to_string(),
        serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
    );

    let built = build_update(
        &config.table,
        table_config,
        &crate::config::types::CrudConfig::default(),
        id,
        &serde_json::Value::Object(body_map),
        driver,
        &RequestContext::default(),
    )?;
    let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "File entry with id '{}' not found",
            id
        )));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
    )
        .into_response())
}

// These functions need access to state, configs, headers, query params, and storage.
// collapsible_if suppressed: early-return pattern would obscure trash logic.
#[allow(clippy::too_many_arguments, clippy::collapsible_if)]
async fn handle_file_store_delete(
    state: &AppState,
    id: &str,
    config: &FileStoreConfig,
    storage: &dyn Storage,
    endpoint: &EndpointConfig,
    pool: &crate::db::pool::DatabasePool,
    table_config: &crate::config::types::TableConfig,
    driver: DatabaseDriver,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    let trash_enabled = config.trash.as_ref().is_some_and(|t| t.enabled);

    if trash_enabled {
        // Check ownership
        if config.ownership.as_ref().is_some_and(|o| !o.admin_override) {
            check_file_store_ownership(state, config, id, endpoint, headers, query_params).await?;
        }

        // Move file to trash if file_path column exists
        let path_check = format!("SELECT file_path FROM {} WHERE id = $1", config.table);
        let row = pool.fetch_optional_json(&path_check, &[id.into()]).await?;
        if let Some(row) = row {
            if let Some(file_path) = row.get("file_path").and_then(|v| v.as_str()) {
                let store_root = storage
                    .root_path()
                    .unwrap_or(PathBuf::from(&config.storage));
                let trash_dest = store_root.join(".trash").join(file_path);
                let source_path = store_root.join(file_path);

                if storage.exists(&source_path).await {
                    if let Some(parent) = trash_dest.parent() {
                        storage.create_dir_all(parent).await.map_err(|e| {
                            AppError::FileOperation(format!(
                                "Failed to create trash directory: {e}"
                            ))
                        })?;
                    }
                    let _ = storage.rename(&source_path, &trash_dest).await;
                }
            }
        }

        let update_sql = format!(
            "UPDATE {} SET deleted_at = NOW() WHERE id = $1",
            config.table
        );
        let rows_affected = pool.execute_with_params(&update_sql, &[id.into()]).await?;

        if rows_affected == 0 {
            return Err(AppError::NotFound(format!(
                "File entry with id '{}' not found",
                id
            )));
        }

        Ok((
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "success": true,
                "message": "File entry soft-deleted (trashed)",
                "rows_affected": rows_affected,
            })),
        )
            .into_response())
    } else {
        // Permanent delete
        if config.ownership.as_ref().is_some_and(|o| !o.admin_override) {
            check_file_store_ownership(state, config, id, endpoint, headers, query_params).await?;
        }

        // Delete file from storage
        let path_check = format!("SELECT file_path FROM {} WHERE id = $1", config.table);
        let row = pool.fetch_optional_json(&path_check, &[id.into()]).await?;
        if let Some(row) = row {
            if let Some(file_path) = row.get("file_path").and_then(|v| v.as_str()) {
                let file_path_buf = storage
                    .root_path()
                    .unwrap_or(PathBuf::from(&config.storage))
                    .join(file_path);
                if storage.exists(&file_path_buf).await {
                    storage.delete(&file_path_buf).await.map_err(|e| {
                        AppError::FileOperation(format!("Failed to delete file: {e}"))
                    })?;
                }
            }
        }

        let built = build_delete(&config.table, table_config, id, driver)?;
        let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;

        if rows_affected == 0 {
            return Err(AppError::NotFound(format!(
                "File entry with id '{}' not found",
                id
            )));
        }

        Ok((
            StatusCode::OK,
            axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
        )
            .into_response())
    }
}

async fn check_file_store_ownership(
    state: &AppState,
    config: &FileStoreConfig,
    id: &str,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<(), AppError> {
    let user_id = extract_user_id(state, endpoint, headers, query_params).await?;
    let pool = {
        let pools = state.db_pools.read().await;
        pools
            .get(&config.database)
            .ok_or_else(|| {
                AppError::Internal(format!("Database '{}' has no pool", config.database))
            })?
            .clone()
    };

    let owner_check = format!("SELECT owner_id FROM {} WHERE id = $1", config.table);
    let row = pool.fetch_optional_json(&owner_check, &[id.into()]).await?;

    if let Some(row) = row {
        let owner_id = row.get("owner_id").and_then(|v| v.as_str());
        if let Some(owner_id) = owner_id
            && owner_id != user_id
        {
            return Err(AppError::Forbidden(
                "Access denied: file entry owned by another user".to_string(),
            ));
        }
    }
    Ok(())
}

async fn extract_user_id(
    state: &AppState,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<String, AppError> {
    // Reuse auth extraction from media handler
    if endpoint.auth == "none" {
        return Ok(String::new());
    }
    let auth_config = state.config.read().await.auth.clone();
    let auth_info = crate::middleware::auth::validate::authenticate(
        &endpoint.auth,
        &auth_config,
        headers,
        query_params,
    )
    .await?;
    Ok(auth_info.subject)
}

async fn apply_row_permissions(
    rows: &[serde_json::Value],
    config: &FileStoreConfig,
) -> Result<Vec<serde_json::Value>, AppError> {
    let permissions = match &config.field_permissions {
        Some(p) => p,
        None => return Ok(rows.to_vec()),
    };

    let mut filtered_rows = Vec::new();
    for row in rows {
        if let Some(obj) = row.as_object() {
            let mut filtered = serde_json::Map::new();
            for (key, value) in obj {
                if is_field_readable(key, permissions).await {
                    filtered.insert(key.clone(), value.clone());
                }
            }
            filtered_rows.push(serde_json::Value::Object(filtered));
        } else {
            filtered_rows.push(row.clone());
        }
    }

    Ok(filtered_rows)
}

async fn is_field_readable(
    field: &str,
    permissions: &HashMap<String, crate::config::types::FileStoreFieldPermissions>,
) -> bool {
    let auto_readonly = ["id", "created_at", "updated_at", "owner_id"];
    if auto_readonly.contains(&field) {
        return true;
    }

    if let Some(field_perm) = permissions.get(field) {
        if field_perm.read.iter().any(|r| r == "*") {
            return true;
        }
        return false;
    }

    false
}

fn extract_file_id(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    segments.last().map(|s| s.to_string())
}

async fn get_db_context(
    state: &AppState,
    config: &FileStoreConfig,
) -> Result<
    (
        crate::db::pool::DatabasePool,
        crate::config::types::TableConfig,
        DatabaseDriver,
    ),
    AppError,
> {
    let pool = {
        let pools = state.db_pools.read().await;
        pools
            .get(config.database.as_str())
            .ok_or_else(|| {
                AppError::Internal(format!("Database '{}' has no pool", config.database))
            })?
            .clone()
    };
    let driver = pool.driver();

    let table_config = {
        let config_guard = state.config.read().await;
        config_guard
            .tables
            .iter()
            .find(|t| t.name == config.table && t.database == config.database)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "Table '{}' in database '{}' not found in config",
                    config.table, config.database
                ))
            })?
            .clone()
    };

    Ok((pool, table_config, driver))
}

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
use crate::db::query::select_one::{build_select_by_id, build_select_file_path};
use crate::db::query::types::{MutationContext, SelectContext};
use crate::db::query::update::build_set_deleted_at;
use crate::error::AppError;
use crate::handlers::common::helpers::extract_id;
use crate::handlers::common::utils::{
    DatabaseContext, extract_user_id, get_db_context, get_db_pool,
};
use crate::middleware::auth::extractor::RequestContext;
use crate::server::state::AppState;
use crate::storage::Storage;

/// Context for list operations.
struct ListContext<'a> {
    state: &'a AppState,
    db_context: &'a DatabaseContext,
    config: &'a FileStoreConfig,
    query_params: &'a HashMap<String, String>,
    endpoint: &'a EndpointConfig,
    headers: &'a axum::http::HeaderMap,
}

/// Context for create operations.
struct CreateContext<'a> {
    db_context: &'a DatabaseContext,
    config: &'a FileStoreConfig,
    endpoint: &'a EndpointConfig,
    headers: &'a axum::http::HeaderMap,
    body: &'a serde_json::Value,
    state: &'a AppState,
    query_params: &'a HashMap<String, String>,
}

/// Context for update operations.
struct UpdateContext<'a> {
    state: &'a AppState,
    id: &'a str,
    config: &'a FileStoreConfig,
    db_context: &'a DatabaseContext,
    endpoint: &'a EndpointConfig,
    headers: &'a axum::http::HeaderMap,
    body: &'a serde_json::Value,
    query_params: &'a HashMap<String, String>,
}

/// Context for delete operations.
struct DeleteContext<'a> {
    state: &'a AppState,
    id: &'a str,
    config: &'a FileStoreConfig,
    storage: &'a dyn Storage,
    endpoint: &'a EndpointConfig,
    db_context: &'a DatabaseContext,
    headers: &'a axum::http::HeaderMap,
    query_params: &'a HashMap<String, String>,
}

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

    dispatch_file_store(
        &state,
        method,
        &uri,
        &endpoint,
        headers,
        query.0,
        &body_value,
    )
    .await
}

/// Dispatch file store requests to the appropriate handler based on method and path.
async fn dispatch_file_store(
    state: &AppState,
    method: axum::http::Method,
    uri: &axum::http::Uri,
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

    let db_context = get_db_context(state, config.database.clone(), config.table.clone()).await?;
    match method {
        axum::http::Method::GET => {
            dispatch_file_store_get(
                state,
                &db_context,
                config,
                &path,
                &query_params,
                endpoint,
                &headers,
            )
            .await
        }
        axum::http::Method::POST => {
            dispatch_file_store_post(
                &db_context,
                config,
                endpoint,
                &headers,
                body,
                state,
                &query_params,
                &path,
            )
            .await
        }
        axum::http::Method::PATCH => match extract_id(&path) {
            Some(id) => {
                handle_file_store_update(&UpdateContext {
                    state,
                    id: &id,
                    config,
                    db_context: &db_context,
                    endpoint,
                    headers: &headers,
                    body,
                    query_params: &query_params,
                })
                .await
            }
            None => Err(AppError::BadRequest("File ID required".to_string())),
        },
        axum::http::Method::DELETE => match extract_id(&path) {
            Some(id) => {
                handle_file_store_delete(&DeleteContext {
                    state,
                    id: &id,
                    config,
                    storage: &*storage,
                    endpoint,
                    db_context: &db_context,
                    headers: &headers,
                    query_params: &query_params,
                })
                .await
            }
            None => Err(AppError::BadRequest("File ID required".to_string())),
        },
        _ => Err(AppError::MethodNotAllowed(
            "Method not allowed for file store endpoint".to_string(),
        )),
    }
}

/// Dispatch GET requests for file store.
#[allow(clippy::too_many_arguments)]
async fn dispatch_file_store_get(
    state: &AppState,
    db_context: &DatabaseContext,
    config: &FileStoreConfig,
    path: &str,
    query_params: &HashMap<String, String>,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
) -> Result<Response, AppError> {
    if path.is_empty() || path == "/" || path == config.table {
        handle_file_store_list(&ListContext {
            state,
            db_context,
            config,
            query_params,
            endpoint,
            headers,
        })
        .await
    } else {
        match extract_id(path) {
            Some(id) => handle_file_store_get_one(db_context, config, &id).await,
            None => Err(AppError::BadRequest("File ID required".to_string())),
        }
    }
}

/// Dispatch POST requests for file store.
#[allow(clippy::too_many_arguments)]
async fn dispatch_file_store_post(
    db_context: &DatabaseContext,
    config: &FileStoreConfig,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    body: &serde_json::Value,
    state: &AppState,
    query_params: &HashMap<String, String>,
    path: &str,
) -> Result<Response, AppError> {
    if path.is_empty() || path == "/" {
        handle_file_store_create(&CreateContext {
            db_context,
            config,
            endpoint,
            headers,
            body,
            state,
            query_params,
        })
        .await
    } else {
        Err(AppError::MethodNotAllowed(
            "POST not allowed on this path".to_string(),
        ))
    }
}

// =============================================================================
// CRUD handlers
// =============================================================================

/// Handle listing file store entries.
async fn handle_file_store_list(ctx: &ListContext<'_>) -> Result<Response, AppError> {
    let pool = &ctx.db_context.pool;
    let driver = ctx.db_context.driver;
    let mut qp = extract_query_params(ctx.query_params);

    qp.page
        .get_or_insert(ctx.config.pagination.default_page_size);
    qp.page_size
        .get_or_insert(ctx.config.pagination.default_page_size);

    if qp.sort.is_none() && !ctx.config.sorting.default_field.is_empty() {
        qp.sort = Some(ctx.config.sorting.default_field.clone());
    }
    if qp.order.is_none() {
        qp.order = Some(ctx.config.sorting.default_order);
    }

    if ctx
        .config
        .ownership
        .as_ref()
        .is_some_and(|o| !o.admin_override)
    {
        qp.filters.insert(
            "owner_id".to_string(),
            extract_user_id(ctx.state, ctx.endpoint, ctx.headers, ctx.query_params).await?,
        );
    }

    let select_ctx = SelectContext::permissive();
    let built = build_select_list(ctx.db_context, &select_ctx, &qp, &RequestContext::default())?;
    let rows = pool
        .fetch_all_json(&built.sql, &built.params)
        .await?;

    let count_built = build_select_list_count(
        &ctx.config.table,
        driver,
        &qp,
        &SelectContext::permissive(),
        &RequestContext::default(),
    )?;
    let count_row = pool
        .fetch_optional_json(&count_built.sql, &count_built.params)
        .await?;
    let total: i64 = count_row
        .as_ref()
        .and_then(|r| r.get("count"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let rows = apply_row_permissions(&rows, ctx.config).await?;

    let page = qp.page.unwrap_or(ctx.config.pagination.default_page_size);
    let page_size = qp
        .page_size
        .unwrap_or(ctx.config.pagination.default_page_size);

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

/// Handle getting a single file store entry.
async fn handle_file_store_get_one(
    db_context: &DatabaseContext,
    config: &FileStoreConfig,
    id: &str,
) -> Result<Response, AppError> {
    let built = build_select_one(
        db_context,
        &SelectContext::permissive(),
        id,
        &RequestContext::default(),
    )?;
    match db_context
        .pool
        .fetch_optional_json(&built.sql, &built.params)
        .await?
    {
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

/// Handle creating a file store entry.
async fn handle_file_store_create(ctx: &CreateContext<'_>) -> Result<Response, AppError> {
    let pool = &ctx.db_context.pool;
    let table_config = &ctx.db_context.table_config;
    let user_id = extract_user_id(ctx.state, ctx.endpoint, ctx.headers, ctx.query_params).await?;

    let writable_columns = table_config
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect::<Vec<_>>();
    let mut body_map = serde_json::Map::new();

    if let Some(obj) = ctx.body.as_object() {
        for (k, v) in obj {
            if writable_columns.contains(&k.to_string()) {
                body_map.insert(k.clone(), v.clone());
            }
        }
    }

    if ctx.config.ownership.is_some() {
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
        ctx.db_context,
        &MutationContext::default(),
        &json_body,
        &RequestContext::default(),
    )?;

    if built.sql.contains("RETURNING") {
        let row = pool
            .fetch_optional_json(&built.sql, &built.params)
            .await?;
        Ok((
            StatusCode::CREATED,
            axum::Json(serde_json::json!({ "data": row })),
        )
            .into_response())
    } else {
        let rows_affected = pool
            .execute_with_params(&built.sql, &built.params)
            .await?;
        Ok((
            StatusCode::CREATED,
            axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
        )
            .into_response())
    }
}

/// Handle updating a file store entry.
async fn handle_file_store_update(ctx: &UpdateContext<'_>) -> Result<Response, AppError> {
    let pool = &ctx.db_context.pool;
    let table_config = &ctx.db_context.table_config;
    let driver = &ctx.db_context.driver;
    if ctx
        .config
        .ownership
        .as_ref()
        .is_some_and(|o| !o.admin_override)
    {
        check_file_store_ownership(
            ctx.state,
            ctx.config,
            ctx.id,
            ctx.endpoint,
            ctx.headers,
            ctx.query_params,
            driver,
        )
        .await?;
    }

    let mut body_map = serde_json::Map::new();
    if let Some(obj) = ctx.body.as_object() {
        for (k, v) in obj {
            if table_config
                .columns
                .iter()
                .any(|c| c.name == *k)
            {
                body_map.insert(k.clone(), v.clone());
            }
        }
    }
    body_map.insert(
        "updated_at".to_string(),
        serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
    );

    let mutate_ctx = MutationContext::default();
    let built = build_update(
        ctx.db_context,
        mutate_ctx,
        ctx.id,
        &serde_json::Value::Object(body_map),
        &RequestContext::default(),
        &None,
    )?;
    let rows_affected = pool
        .execute_with_params(&built.sql, &built.params)
        .await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "File entry with id '{}' not found",
            ctx.id
        )));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
    )
        .into_response())
}

/// Handle deleting a file store entry.
async fn handle_file_store_delete(ctx: &DeleteContext<'_>) -> Result<Response, AppError> {
    let pool = &ctx.db_context.pool;
    let driver = &ctx.db_context.driver;
    let trash_enabled = ctx.config.trash.as_ref().is_some_and(|t| t.enabled);

    if trash_enabled {
        if ctx
            .config
            .ownership
            .as_ref()
            .is_some_and(|o| !o.admin_override)
        {
            check_file_store_ownership(
                ctx.state,
                ctx.config,
                ctx.id,
                ctx.endpoint,
                ctx.headers,
                ctx.query_params,
                &driver,
            )
            .await?;
        }

        let built = build_select_file_path(&ctx.config.table, ctx.db_context.driver);
        let row = pool
            .fetch_optional_json(&built.sql, &[ctx.id.into()])
            .await?;
        if let Some(row) = row
            && let Some(file_path) = row.get("file_path").and_then(|v| v.as_str())
        {
            let store_root = ctx
                .storage
                .root_path()
                .unwrap_or(PathBuf::from(&ctx.config.storage));
            let trash_dest = store_root.join(".trash").join(file_path);
            let source_path = store_root.join(file_path);

            if ctx.storage.exists(&source_path).await {
                if let Some(parent) = trash_dest.parent() {
                    ctx.storage.create_dir_all(parent).await.map_err(|e| {
                        AppError::FileOperation(format!("Failed to create trash directory: {e}"))
                    })?;
                }
                let _ = ctx.storage.rename(&source_path, &trash_dest).await;
            }
        }

        let built = build_set_deleted_at(&ctx.config.table, ctx.db_context.driver)
            .map_err(|e| AppError::Internal(format!("Failed to build query: {e}")))?;
        let rows_affected = pool
            .execute_with_params(&built.sql, &[ctx.id.into()])
            .await?;

        if rows_affected == 0 {
            return Err(AppError::NotFound(format!(
                "File entry with id '{}' not found",
                ctx.id
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
        if ctx
            .config
            .ownership
            .as_ref()
            .is_some_and(|o| !o.admin_override)
        {
            check_file_store_ownership(
                ctx.state,
                ctx.config,
                ctx.id,
                ctx.endpoint,
                ctx.headers,
                ctx.query_params,
                &driver,
            )
            .await?;
        }

        let built = build_select_file_path(&ctx.config.table, ctx.db_context.driver);
        let row = pool
            .fetch_optional_json(&built.sql, &[ctx.id.into()])
            .await?;
        if let Some(row) = row
            && let Some(file_path) = row.get("file_path").and_then(|v| v.as_str())
        {
            let file_path_buf = ctx
                .storage
                .root_path()
                .unwrap_or(PathBuf::from(&ctx.config.storage))
                .join(file_path);
            if ctx.storage.exists(&file_path_buf).await {
                ctx.storage
                    .delete(&file_path_buf)
                    .await
                    .map_err(|e| AppError::FileOperation(format!("Failed to delete file: {e}")))?;
            }
        }

        let built = build_delete(
            ctx.id,
            ctx.db_context,
            &RequestContext::default(),
            &None,
        )?;
        let rows_affected = pool
            .execute_with_params(&built.sql, &built.params)
            .await?;

        if rows_affected == 0 {
            return Err(AppError::NotFound(format!(
                "File entry with id '{}' not found",
                ctx.id
            )));
        }

        Ok((
            StatusCode::OK,
            axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
        )
            .into_response())
    }
}

// =============================================================================
// Shared helpers
// =============================================================================

/// Check ownership of a file store entry.
async fn check_file_store_ownership(
    state: &AppState,
    config: &FileStoreConfig,
    id: &str,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
    driver: &DatabaseDriver,
) -> Result<(), AppError> {
    let user_id = extract_user_id(state, endpoint, headers, query_params).await?;
    let pool = get_db_pool(state, &config.database).await?;

    let built = build_select_by_id(&config.table, &["owner_id"], *driver)
        .map_err(|e| AppError::Internal(format!("Failed to build query: {e}")))?;
    let row = pool.fetch_optional_json(&built.sql, &[id.into()]).await?;

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

/// Apply field permissions to response rows.
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

/// Check if a field is readable based on permissions.
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

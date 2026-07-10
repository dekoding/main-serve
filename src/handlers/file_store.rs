/// File store (database-backed file catalog) handler.
use std::collections::HashMap;
use std::path::PathBuf;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::{DatabaseDriver, FileStoreConfig};
use crate::db::query::builders::{
    build_delete, build_file_ref_delete, build_file_ref_insert, build_file_ref_max_order,
    build_file_ref_select, build_insert, build_select_list, build_select_one, build_update,
};
use crate::db::query::helpers::extract_query_params;
use crate::db::query::select_one::{
    build_select_by_id, build_select_file_path, build_select_trashed, build_select_trashed_ids,
    build_select_trashed_item,
};
use crate::db::query::types::{MutationContext, SelectContext};
use crate::db::query::update::{build_set_restored, build_set_trashed};
use crate::error::AppError;
use crate::handlers::common::helpers::{extract_file_path, extract_id};
use crate::handlers::common::store::resolve_store;
use crate::handlers::common::utils::{DatabaseContext, HandlerContext, filter_writable_body};
use crate::middleware::auth::extractor::RequestContext;
use crate::server::state::AppState;
use crate::storage::Storage;

/// Shared context for file store CRUD operations.
struct FileStoreContext<'a> {
    handler_ctx: &'a HandlerContext<'a>,
    db_ctx: &'a DatabaseContext,
    config: &'a FileStoreConfig,
    id: Option<&'a str>,
    body: Option<&'a serde_json::Value>,
    storage: Option<&'a dyn Storage>,
}

impl FileStoreContext<'_> {
    /// Extract the file ID, returning an error if not present.
    fn require_id(&self) -> Result<&str, AppError> {
        self.id
            .ok_or_else(|| AppError::Internal("File ID required".to_string()))
    }

    /// Extract the storage backend, returning an error if not present.
    fn require_storage(&self) -> Result<&dyn Storage, AppError> {
        self.storage
            .ok_or_else(|| AppError::Internal("Storage backend required".to_string()))
    }
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

    let handler_ctx = HandlerContext {
        state: &state,
        endpoint: &endpoint,
        headers: &headers,
        query_params: &query.0,
    };

    dispatch_file_store(&handler_ctx, method, &uri, &body_value).await
}

/// Dispatch file store requests to the appropriate handler based on method and path.
async fn dispatch_file_store(
    handler_ctx: &HandlerContext<'_>,
    method: axum::http::Method,
    uri: &axum::http::Uri,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let config = &handler_ctx
        .endpoint
        .file_store
        .as_ref()
        .ok_or_else(|| AppError::NotFound("File store config not found".to_string()))?;

    let state = handler_ctx.state;
    let path = uri.path().to_string();

    // Handle admin trash routes
    if path.starts_with("/_main-serve/file-store/trash") {
        let (storage, root) = resolve_store(state, &config.storage)?;
        let db_ctx = state
            .get_db_context(&config.database, &config.table)
            .await?;
        return handle_file_store_trash(method, &path, config, &*storage, &root, &db_ctx).await;
    }

    let (storage, _) = resolve_store(state, &config.storage)?;

    let db_ctx = state
        .get_db_context(&config.database, &config.table)
        .await?;
    match method {
        axum::http::Method::GET => {
            dispatch_file_store_get(handler_ctx, &db_ctx, config, &path).await
        }
        axum::http::Method::POST => {
            dispatch_file_store_post(handler_ctx, &db_ctx, config, body, &path).await
        }
        axum::http::Method::PATCH => match extract_id(&path) {
            Some(id) => {
                handle_file_store_update(&FileStoreContext {
                    handler_ctx,
                    id: Some(&id),
                    config,
                    db_ctx: &db_ctx,
                    body: Some(body),
                    storage: None,
                })
                .await
            }
            None => Err(AppError::BadRequest(format!(
                "File ID required for path: {}",
                path
            ))),
        },
        axum::http::Method::DELETE => {
            // Handle content reference detach: /api/files/{id}/refs/{entity_id}
            if let Some(full_path) = path.strip_prefix("/").and_then(|p| {
                p.find("/refs/").map(|refs_pos| {
                    let before_refs = &p[..refs_pos];
                    let after_refs = &p[refs_pos + 6..];
                    (before_refs, after_refs)
                })
            }) {
                let (before_refs, after_refs) = full_path;
                if let Some(id) = extract_id(before_refs) {
                    let entity_id = after_refs.split('/').next().unwrap_or("");
                    if !entity_id.is_empty() {
                        return handle_file_store_detach(&id, entity_id, config, &db_ctx.pool)
                            .await;
                    }
                }
            }

            match extract_id(&path) {
                Some(id) => {
                    handle_file_store_delete(&FileStoreContext {
                        handler_ctx,
                        id: Some(&id),
                        config,
                        storage: Some(&*storage),
                        db_ctx: &db_ctx,
                        body: None,
                    })
                    .await
                }
                None => Err(AppError::BadRequest("File ID required".to_string())),
            }
        }
        _ => Err(AppError::MethodNotAllowed(
            "Method not allowed for file store endpoint".to_string(),
        )),
    }
}

async fn dispatch_file_store_get(
    handler_ctx: &HandlerContext<'_>,
    db_ctx: &DatabaseContext,
    config: &FileStoreConfig,
    path: &str,
) -> Result<Response, AppError> {
    let endpoint_path = handler_ctx.endpoint.path.trim_end_matches('/');
    let base_path = endpoint_path
        .strip_suffix("/{id}")
        .or_else(|| endpoint_path.strip_suffix("/*"))
        .unwrap_or(endpoint_path)
        .trim_end_matches('/');

    let path_normalized = path.trim_end_matches('/');

    if path_normalized.is_empty() || path_normalized == "/" || path_normalized == base_path {
        handle_file_store_list(&FileStoreContext {
            handler_ctx,
            db_ctx,
            config,
            id: None,
            body: None,
            storage: None,
        })
        .await
    } else {
        match extract_id(path) {
            Some(id) => handle_file_store_get_one(handler_ctx, db_ctx, config, &id).await,
            None => Err(AppError::BadRequest("File ID required".to_string())),
        }
    }
}

/// Dispatch POST requests for file store.
async fn dispatch_file_store_post(
    handler_ctx: &HandlerContext<'_>,
    db_ctx: &DatabaseContext,
    config: &FileStoreConfig,
    body: &serde_json::Value,
    path: &str,
) -> Result<Response, AppError> {
    // Get the endpoint path for comparison
    let endpoint_path = handler_ctx.endpoint.path.trim_end_matches('/');
    // Remove trailing /* or {*/rest} if present
    let base_path = endpoint_path
        .strip_suffix("/*")
        .or_else(|| endpoint_path.strip_suffix("/{*rest}"))
        .unwrap_or(endpoint_path)
        .trim_end_matches('/');

    // Handle content reference attach: /api/files/{id}/refs
    if path.ends_with("/refs") || path.ends_with("/refs/") {
        let path_without_refs = path
            .strip_suffix("/refs")
            .or_else(|| path.strip_suffix("/refs/"))
            .unwrap_or("");
        if let Some(id) = extract_id(path_without_refs) {
            return handle_file_store_attach(&id, config, &db_ctx.pool, body).await;
        }
    }

    if path.is_empty() || path == "/" || path == base_path {
        handle_file_store_create(&FileStoreContext {
            handler_ctx,
            db_ctx,
            config,
            body: Some(body),
            id: None,
            storage: None,
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
async fn handle_file_store_list(ctx: &FileStoreContext<'_>) -> Result<Response, AppError> {
    let pool = &ctx.db_ctx.pool;
    let mut qp = extract_query_params(ctx.handler_ctx.query_params);

    qp.page.get_or_insert(1);
    qp.page_size
        .get_or_insert(ctx.config.pagination.default_page_size);

    if qp.sort.is_none() && !ctx.config.sorting.default_field.is_empty() {
        qp.sort = Some(ctx.config.sorting.default_field.clone());
    }
    if qp.order.is_none() {
        qp.order = Some(ctx.config.sorting.default_order);
    }

    if let Some(ownership) = &ctx.config.ownership {
        let user_id = ctx.handler_ctx.extract_user_id().await?;
        let auth_info = ctx.handler_ctx.extract_auth_info().await?;

        if !ctx.handler_ctx.endpoint.roles.is_admin(&auth_info.role) {
            let owner_col = ownership.owner_column.as_str();
            qp.filters.insert(owner_col.to_string(), user_id);
        }
    }

    let select_ctx = SelectContext::permissive();
    let built = build_select_list(ctx.db_ctx, &select_ctx, &qp, &RequestContext::default())?;
    let mut rows = pool.fetch_all_json(&built.sql, &built.params).await?;

    // Filter out trashed items
    rows.retain(|row| row.get("trashed_at").and_then(|v| v.as_str()).is_none());

    let total: i64 = rows.len() as i64;

    // Convert numeric ids to strings for consistency with create/get responses
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

    tracing::debug!(
        "LIST rows count={}, first_row={:?}",
        rows.len(),
        rows.first()
    );

    Ok((StatusCode::OK, axum::Json(response)).into_response())
}

/// Handle getting a single file store entry.
async fn handle_file_store_get_one(
    handler_ctx: &HandlerContext<'_>,
    db_ctx: &DatabaseContext,
    config: &FileStoreConfig,
    id: &str,
) -> Result<Response, AppError> {
    // Check ownership if configured
    if let Some(ownership) = &config.ownership {
        let auth_info = handler_ctx.extract_auth_info().await?;
        let is_admin = handler_ctx.endpoint.roles.is_admin(&auth_info.role);
        if ownership.admin_override && !is_admin {
            check_file_store_ownership(handler_ctx, config, id, &db_ctx.pool.driver()).await?;
        }
    }

    let built = build_select_one(
        db_ctx,
        &SelectContext::permissive(),
        id,
        &RequestContext::default(),
    )?;
    match db_ctx
        .pool
        .fetch_optional_json(&built.sql, &built.params)
        .await?
    {
        Some(mut row) => {
            if let Some(permissions) = &config.field_permissions {
                let auth_info = handler_ctx.extract_auth_info().await?;
                let user_role = auth_info.role;
                let user_roles = user_roles(&user_role);
                let mut filtered = serde_json::Map::new();
                if let Some(obj) = row.as_object_mut() {
                    let keys: Vec<String> = obj.keys().cloned().collect();
                    for key in keys {
                        if is_field_readable_by_role(&key, &user_roles, permissions)
                            && let Some(value) = obj.remove(&key)
                        {
                            filtered.insert(key, value);
                        }
                    }
                }
                row = serde_json::Value::Object(filtered);
            }
            // Convert id to string if it's a number (consistent with CREATE response)
            if let serde_json::Value::Object(ref mut obj) = row
                && let Some(id_val) = obj.get("id")
            {
                let id_str = id_val
                    .as_i64()
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| id_val.to_string());
                obj.insert("id".to_string(), serde_json::Value::String(id_str));
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
async fn handle_file_store_create(ctx: &FileStoreContext<'_>) -> Result<Response, AppError> {
    let pool = &ctx.db_ctx.pool;
    let table_config = &ctx.db_ctx.table_config;
    let user_id = ctx.handler_ctx.extract_user_id().await?;

    // Check field write permissions
    if let Some(ref permissions) = ctx.config.field_permissions {
        let auth_info = ctx.handler_ctx.extract_auth_info().await?;
        let user_role = auth_info.role;
        let user_roles = user_roles(&user_role);
        if let Some(obj) = ctx.body.and_then(|v| v.as_object()) {
            for (k, _) in obj {
                if table_config.columns.iter().any(|c| c.name == *k)
                    && !is_field_writable(k, &user_roles, permissions)
                {
                    return Err(AppError::Forbidden(format!(
                        "Field '{}' is not writable by the current user's roles",
                        k
                    )));
                }
            }
        }
    }

    let writable_columns = table_config
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect::<Vec<_>>();
    let body_value = ctx
        .body
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    let mut body_map = filter_writable_body(&body_value, &writable_columns);

    if ctx.config.ownership.is_some() {
        let owner_col = ctx
            .config
            .ownership
            .as_ref()
            .map(|o| o.owner_column.as_str())
            .unwrap_or("owner_id");
        body_map.insert(
            owner_col.to_string(),
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
        ctx.db_ctx,
        &MutationContext {
            writable_fields: vec!["*".to_string()],
            ..MutationContext::default()
        },
        &json_body,
        &RequestContext::default(),
    )?;

    if built.sql.contains("RETURNING") {
        let mut row = pool.fetch_optional_json(&built.sql, &built.params).await?;
        // Add owner column to response if ownership is configured
        if let Some(serde_json::Value::Object(ref mut obj)) = row
            && let Some(ownership) = &ctx.config.ownership
        {
            let owner_col = ownership.owner_column.as_str();
            obj.insert(
                owner_col.to_string(),
                serde_json::Value::String(user_id.clone()),
            );
        }
        // Convert id to string if it's a number
        if let Some(serde_json::Value::Object(ref mut obj)) = row
            && let Some(id_val) = obj.get("id")
        {
            let id_str = id_val
                .as_i64()
                .map(|i| i.to_string())
                .unwrap_or_else(|| id_val.to_string());
            obj.insert("id".to_string(), serde_json::Value::String(id_str));
        }
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

/// Handle updating a file store entry.
async fn handle_file_store_update(ctx: &FileStoreContext<'_>) -> Result<Response, AppError> {
    let pool = &ctx.db_ctx.pool;
    let table_config = &ctx.db_ctx.table_config;
    let driver = &ctx.db_ctx.pool.driver();
    let auth_info = ctx.handler_ctx.extract_auth_info().await?;

    if ctx.config.ownership.is_some() && !ctx.handler_ctx.endpoint.roles.is_admin(&auth_info.role) {
        check_file_store_ownership(ctx.handler_ctx, ctx.config, ctx.require_id()?, driver).await?;
    }

    // Check field write permissions
    if let Some(ref permissions) = ctx.config.field_permissions {
        let user_role = auth_info.role;
        let user_roles = user_roles(&user_role);
        if let Some(obj) = ctx.body.and_then(|v| v.as_object()) {
            for (k, _) in obj {
                if table_config.columns.iter().any(|c| c.name == *k)
                    && k != "updated_at"
                    && !is_field_writable(k, &user_roles, permissions)
                {
                    return Err(AppError::Forbidden(format!(
                        "Field '{}' is not writable by the current user's roles",
                        k
                    )));
                }
            }
        }
    }

    let writable_columns: Vec<String> = table_config
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect();
    let body_value = ctx
        .body
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    let mut body_map = filter_writable_body(&body_value, &writable_columns);
    body_map.insert(
        "updated_at".to_string(),
        serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
    );

    let mutate_ctx = MutationContext {
        writable_fields: vec!["*".to_string()],
        ..MutationContext::default()
    };
    let built = build_update(
        ctx.db_ctx,
        mutate_ctx,
        ctx.require_id()?,
        &serde_json::Value::Object(body_map),
        &RequestContext::default(),
        &None,
    )?;
    let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "File entry with id '{}' not found",
            ctx.require_id()?
        )));
    }

    // Return the updated row
    let built = build_select_one(
        ctx.db_ctx,
        &SelectContext::permissive(),
        ctx.require_id()?,
        &RequestContext::default(),
    )?;
    let row = pool.fetch_optional_json(&built.sql, &built.params).await?;

    let response = match row {
        Some(row) => axum::Json(row),
        None => axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
    };

    Ok((StatusCode::OK, response).into_response())
}

/// Handle deleting a file store entry.
async fn handle_file_store_delete(ctx: &FileStoreContext<'_>) -> Result<Response, AppError> {
    let pool = &ctx.db_ctx.pool;
    let driver = &ctx.db_ctx.pool.driver();
    let storage = ctx.require_storage()?;
    let id = ctx.require_id()?;
    let trash_enabled = ctx.config.trash.as_ref().is_some_and(|t| t.enabled);

    if trash_enabled {
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
            let store_root = storage
                .root_path()
                .unwrap_or(PathBuf::from(&ctx.config.storage));
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
                .unwrap_or(PathBuf::from(&ctx.config.storage))
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

// =============================================================================
// Shared helpers
// =============================================================================

/// Check ownership of a file store entry.
async fn check_file_store_ownership(
    handler_ctx: &HandlerContext<'_>,
    config: &FileStoreConfig,
    id: &str,
    driver: &DatabaseDriver,
) -> Result<(), AppError> {
    let user_id = handler_ctx.extract_user_id().await?;
    let pool = handler_ctx.state.db_pool(&config.database).await?;

    let owner_col = config
        .ownership
        .as_ref()
        .map(|o| o.owner_column.as_str())
        .unwrap_or("owner_id");

    let built = build_select_by_id(&config.table, &[owner_col], *driver)
        .map_err(|e| AppError::Internal(format!("Failed to build query: {e}")))?;
    let row = pool.fetch_optional_json(&built.sql, &[id.into()]).await?;

    if let Some(row) = row {
        let owner_id = row.get(owner_col).and_then(|v| v.as_str());
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
    let mut filtered_rows = Vec::new();
    for row in rows {
        if let Some(obj) = row.as_object() {
            let mut filtered = serde_json::Map::new();
            for (key, value) in obj {
                let readable = is_field_readable(key, config).await;
                if readable {
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

/// Check if a field is writable based on permissions.
fn is_field_writable(
    field: &str,
    user_roles: &[String],
    permissions: &HashMap<String, crate::config::types::FileStoreFieldPermissions>,
) -> bool {
    if let Some(field_perm) = permissions.get(field) {
        if field_perm.write.iter().any(|r| r == "*") {
            return true;
        }
        return user_roles.iter().any(|r| field_perm.write.contains(r));
    }
    true
}

/// Get a list of user roles from the auth info.
fn user_roles(role: &Option<String>) -> Vec<String> {
    match role {
        Some(r) => vec![r.clone()],
        None => vec![],
    }
}

/// Check if a field is readable based on permissions.
async fn is_field_readable(field: &str, config: &FileStoreConfig) -> bool {
    let owner_col = config
        .ownership
        .as_ref()
        .map(|o| o.owner_column.as_str())
        .unwrap_or("owner_id");
    let auto_readonly = ["id", "created_at", "updated_at", owner_col];
    if auto_readonly.contains(&field) {
        return true;
    }

    if let Some(permissions) = &config.field_permissions
        && let Some(field_perm) = permissions.get(field)
        && field_perm.read.iter().any(|r| r == "*")
    {
        return true;
    }

    // No explicit permissions or permissions without "*" = readable by default
    // (We can't check user role here, so be permissive)
    true
}

/// Check if a field is readable based on the user's roles.
fn is_field_readable_by_role(
    field: &str,
    user_roles: &[String],
    permissions: &HashMap<String, crate::config::types::FileStoreFieldPermissions>,
) -> bool {
    if let Some(field_perm) = permissions.get(field) {
        if field_perm.read.iter().any(|r| r == "*") {
            return true;
        }
        return user_roles.iter().any(|r| field_perm.read.contains(r));
    }
    true
}

// =============================================================================
// Content references
// =============================================================================

/// Handle content reference attach for file store.
async fn handle_file_store_attach(
    id: &str,
    config: &FileStoreConfig,
    pool: &crate::db::pool::DatabasePool,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let content_refs = config
        .content_references
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Content references not enabled".to_string()))?;

    if !content_refs.enabled {
        return Err(AppError::MethodNotAllowed(
            "Content references are disabled".to_string(),
        ));
    }

    let entity_id = body
        .get("entity_id")
        .ok_or_else(|| AppError::BadRequest("entity_id is required".to_string()))?;
    let entity_id = entity_id
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| entity_id.as_i64().map(|i| i.to_string()))
        .or_else(|| entity_id.as_u64().map(|i| i.to_string()))
        .ok_or_else(|| AppError::BadRequest("entity_id is required".to_string()))?;

    let content_type = body
        .get("content_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("content_type is required".to_string()))?;

    let table = &content_refs.table;
    let file_id_col = &content_refs.media_id_column;
    let entity_id_col = &content_refs.entity_id_column;
    let content_type_col = &content_refs.content_type_column;
    let order_col = &content_refs.order_column;

    let built = build_file_ref_max_order(table, order_col, pool.driver());
    let max_row = pool.fetch_optional_json(&built.sql, &[]).await?;
    let max_order: i64 = max_row
        .as_ref()
        .and_then(|r| r.get("max_order").and_then(|v| v.as_i64()))
        .unwrap_or(0);

    let next_order = max_order + 1;

    let built = build_file_ref_insert(
        table,
        &[
            file_id_col.as_str(),
            entity_id_col.as_str(),
            content_type_col.as_str(),
            order_col.as_str(),
        ],
        pool.driver(),
    );

    pool.execute_with_params(
        &built.sql,
        &[
            id.into(),
            serde_json::Value::String(entity_id.clone()),
            serde_json::Value::String(content_type.to_string()),
            serde_json::Value::Number(serde_json::Number::from(next_order)),
        ],
    )
    .await?;

    let built = build_file_ref_select(
        table,
        file_id_col,
        entity_id_col,
        content_type_col,
        pool.driver(),
    );
    let row = pool
        .fetch_optional_json(
            &built.sql,
            &[id.into(), entity_id.into(), content_type.to_string().into()],
        )
        .await?;

    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({ "data": row })),
    )
        .into_response())
}

/// Handle content reference detach for file store.
async fn handle_file_store_detach(
    id: &str,
    entity_id: &str,
    config: &FileStoreConfig,
    pool: &crate::db::pool::DatabasePool,
) -> Result<Response, AppError> {
    let content_refs = config
        .content_references
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Content references not enabled".to_string()))?;

    if !content_refs.enabled {
        return Err(AppError::MethodNotAllowed(
            "Content references are disabled".to_string(),
        ));
    }

    let table = &content_refs.table;
    let file_id_col = &content_refs.media_id_column;
    let entity_id_col = &content_refs.entity_id_column;
    let content_type_col = &content_refs.content_type_column;

    let built = build_file_ref_delete(
        table,
        file_id_col,
        entity_id_col,
        content_type_col,
        pool.driver(),
    );
    let rows_affected = pool
        .execute_with_params(
            &built.sql,
            &[id.into(), serde_json::Value::String(entity_id.to_string())],
        )
        .await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "No content reference found for file id '{}', entity '{}'",
            id, entity_id
        )));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": format!("Detached file '{}' from entity '{}'", id, entity_id),
            "rows_affected": rows_affected,
        })),
    )
        .into_response())
}

// =============================================================================
// Trash management
// =============================================================================

/// Handle file store trash management routes.
async fn handle_file_store_trash(
    method: axum::http::Method,
    path: &str,
    config: &FileStoreConfig,
    storage: &dyn Storage,
    root: &std::path::Path,
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
    root: &std::path::Path,
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

    let file_path = extract_file_path(row, id).await.unwrap_or_default();

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
    root: &std::path::Path,
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
            let built = crate::db::query::builders::build_delete(
                id_val,
                db_ctx,
                &RequestContext::default(),
                &None,
            )?;
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
    root: &std::path::Path,
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

    let file_path = extract_file_path(row, id).await.unwrap_or_default();

    if !file_path.is_empty() {
        let trash_path = root.join(&trash_config.prefix).join(&file_path);
        if storage.exists(&trash_path).await {
            let _ = storage.delete(&trash_path).await;
        }
    }

    let built =
        crate::db::query::builders::build_delete(id, db_ctx, &RequestContext::default(), &None)?;
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

/// File store (database-backed file catalog) handler.
pub mod content_refs;
pub mod create;
pub mod delete;
pub mod get_one;
pub mod list;
pub mod trash;
pub mod update;

use std::collections::HashMap;

use axum::response::Response;

use crate::config::types::{DatabaseDriver, FileStoreConfig};
use crate::db::query::select_one::build_select_by_id;
use crate::error::AppError;
use crate::handlers::common::helpers::extract_id;
use crate::handlers::common::store::resolve_store;
use crate::handlers::common::utils::{DatabaseContext, HandlerContext};
use crate::server::state::AppState;
use crate::storage::Storage;

/// Shared context for file store CRUD operations.
pub struct FileStoreContext<'a> {
    pub handler_ctx: &'a HandlerContext<'a>,
    pub db_ctx: &'a DatabaseContext,
    pub config: &'a FileStoreConfig,
    pub id: Option<&'a str>,
    pub body: Option<&'a serde_json::Value>,
    pub storage: Option<&'a dyn Storage>,
}

impl FileStoreContext<'_> {
    /// Extract the file ID, returning an error if not present.
    pub fn require_id(&self) -> Result<&str, AppError> {
        self.id
            .ok_or_else(|| AppError::Internal("File ID required".to_string()))
    }

    /// Extract the storage backend, returning an error if not present.
    pub fn require_storage(&self) -> Result<&dyn Storage, AppError> {
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
        return trash::handle_file_store_trash(method, &path, config, &*storage, &root, &db_ctx)
            .await;
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
                update::handle_file_store_update(&FileStoreContext {
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
                        return content_refs::handle_file_store_detach(
                            &id,
                            entity_id,
                            config,
                            &db_ctx.pool,
                        )
                        .await;
                    }
                }
            }

            match extract_id(&path) {
                Some(id) => {
                    delete::handle_file_store_delete(&FileStoreContext {
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
        list::handle_file_store_list(&FileStoreContext {
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
            Some(id) => get_one::handle_file_store_get_one(handler_ctx, db_ctx, config, &id).await,
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
            return content_refs::handle_file_store_attach(&id, config, &db_ctx.pool, body).await;
        }
    }

    if path.is_empty() || path == "/" || path == base_path {
        create::handle_file_store_create(&FileStoreContext {
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
pub fn is_field_writable(
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
pub fn user_roles(role: &Option<String>) -> Vec<String> {
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
pub fn is_field_readable_by_role(
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

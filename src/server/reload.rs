use axum::Json;
/// Hot-reload and health check endpoint handlers.
///
/// These are hard-coded system endpoints that are always present:
/// - `GET /_main-serve/health` - unauthenticated health check
/// - `POST /_main-serve/reload` - authenticated config reload
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde_json::json;
use subtle::ConstantTimeEq;

use std::sync::Arc;

use super::state::{AppState, compute_store_changes};
use crate::config::load_config;
use crate::config::schema_registry::SchemaRegistry;
use crate::db::migration::{ensure_media_columns, run_migrations};
use crate::db::pool::{close_pools, create_pools};
use crate::storage::create_store;

/// Health check handler.
///
/// Returns `200 OK` with status, database, and endpoint information.
/// No authentication required.
///
/// Response body:
/// ```json
/// {
///   "status": "healthy",
///   "databases_configured": 1,
///   "databases_connected": 1,
///   "endpoints_configured": 5,
///   "stores_configured": 1
/// }
/// ```
pub async fn handle_health(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.read().await;
    let db_count = config.databases.len();
    let endpoint_count = config.endpoints.len();
    let store_count = config.stores.len();
    drop(config);

    let pools = state.db_pools.read().await;
    let connected_dbs = pools.len();
    drop(pools);

    Json(json!({
        "status": "healthy",
        "databases_configured": db_count,
        "databases_connected": connected_dbs,
        "endpoints_configured": endpoint_count,
        "stores_configured": store_count,
    }))
}

/// Hot-reload handler.
///
/// Re-reads the YAML config file, validates it, and atomically swaps the config.
/// Recreates database pools and runs migrations for the new config.
/// Requires `Authorization: Bearer <admin_token>`.
///
/// # Errors
///
/// Returns a `(StatusCode, Json)` error tuple if the admin token is missing
/// or invalid, or if config loading/migration fails.
pub async fn handle_reload(
    State(mut state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // Authenticate the request.
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match token {
        Some(t) if t.as_bytes().ct_eq(state.admin_token.as_bytes()).into() => {}
        _ => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": {
                        "code": "unauthorized",
                        "message": "Invalid or missing admin token"
                    }
                })),
            ));
        }
    }

    // Load and validate the new config.
    let new_config = match load_config(&state.config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Config reload failed: {e}");
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "code": "reload_failed",
                        "message": format!("Config reload failed: {e}")
                    }
                })),
            ));
        }
    };

    let endpoint_count = new_config.endpoints.len();
    let db_count = new_config.databases.len();
    let table_count = new_config.tables.len();

    // Compute store changes before the config/pools swap, so we still have
    // access to new_config for the swap below.
    let old_store_configs = state.store_configs.read().await.clone();
    let (unchanged, changed_or_removed): (Vec<String>, Vec<String>) =
        compute_store_changes(&old_store_configs, &new_config.stores);

    // Recreate database pools for the new config.
    let new_pools = match create_pools(&new_config.databases).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Database pool creation failed during reload: {e}");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "code": "reload_failed",
                        "message": format!("Database pool creation failed: {e}")
                    }
                })),
            ));
        }
    };

    // Run migrations against the new pools.
    if let Err(e) = run_migrations(&new_config.tables, &new_pools, &new_config.databases).await {
        tracing::error!("Migration failed during reload: {e}");
        close_pools(&new_pools).await;
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": {
                    "code": "reload_failed",
                    "message": format!("Migration failed: {e}")
                }
            })),
        ));
    }

    // Ensure media-specific columns exist on media tables.
    if let Err(e) = ensure_media_columns(&new_config.endpoints, &new_pools).await {
        tracing::error!("Failed to ensure media columns during reload: {e}");
        close_pools(&new_pools).await;
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": {
                    "code": "reload_failed",
                    "message": format!("Failed to ensure media columns: {e}")
                }
            })),
        ));
    }

    // Handle store changes: create new/updated stores BEFORE swapping config/pools.
    // This ensures failures are handled gracefully without partial state swaps.
    let new_store_configs = new_config.stores.clone();
    let (_old_stores, new_stores): (Vec<Arc<dyn crate::storage::Storage>>, _) = {
        if changed_or_removed.is_empty() {
            (Vec::new(), Arc::clone(&state.stores))
        } else {
            // Collect old store references (no longer needed after pool drain inlining).
            let _old_stores: Vec<Arc<dyn crate::storage::Storage>> = changed_or_removed
                .iter()
                .filter_map(|name| state.stores.get(name).cloned())
                .collect();

            // Build new stores map by copying existing and removing/replacing changed ones
            let mut map = (*state.stores).clone();
            for name in &changed_or_removed {
                map.remove(name);
            }
            for (name, store_config) in &new_store_configs {
                if !unchanged.contains(name) {
                    let store = create_store(store_config).await.map_err(|e| {
                        tracing::error!("Store creation failed during reload: {name}: {e}");
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "error": {
                                    "code": "reload_failed",
                                    "message": format!("Store creation failed: {e}")
                                }
                            })),
                        )
                    })?;
                    map.insert(name.clone(), store);
                }
            }
            (_old_stores, Arc::new(map))
        }
    };

    // Atomically swap config, pools, stores, and store_configs together.
    // Hold all write locks simultaneously so no request can see mismatched state.
    let old_pools = {
        let mut config = state.config.write().await;
        let mut pools = state.db_pools.write().await;
        let mut store_configs = state.store_configs.write().await;
        let old_pools = std::mem::replace(&mut *pools, new_pools);
        *config = new_config;
        *store_configs = new_store_configs;
        drop(store_configs);
        state.stores = new_stores;
        old_pools
    };

    // Rebuild the schema registry for the new config. The config swap already
    // happened above, so this reads the updated tables and global_schemas.
    {
        let mut registry = state.schema_registry.write().await;
        let config = state.config.read().await;
        *registry = SchemaRegistry::new(&config, &state.config_path).map_err(|e| {
            tracing::error!("Schema registry rebuild failed during reload: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "code": "reload_failed",
                        "message": format!("Schema registry rebuild failed: {e}")
                    }
                })),
            )
        })?;
    }

    // Log old store drain (no actual work needed).

    // Drain old pools *after* releasing the locks so in-flight requests on the
    // old pools can finish naturally. sqlx pool close() waits for active
    // connections to be returned, so this is safe.
    close_pools(&old_pools).await;
    tracing::debug!("Old database pools drained.");

    tracing::info!(
        "Configuration reloaded successfully: {endpoint_count} endpoints, {db_count} databases, {table_count} tables"
    );

    Ok(Json(json!({
        "status": "reloaded",
        "summary": {
            "endpoints": endpoint_count,
            "databases": db_count,
            "tables": table_count,
        }
    })))
}

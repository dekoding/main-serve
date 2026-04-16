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

use super::state::AppState;
use crate::config::load_config;
use crate::db::migration::run_migrations;
use crate::db::pool::{close_pools, create_pools};

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
///   "endpoints_configured": 5
/// }
/// ```
pub async fn handle_health(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.read().await;
    let db_count = config.databases.len();
    let endpoint_count = config.endpoints.len();
    drop(config);

    let pools = state.db_pools.read().await;
    let connected_dbs = pools.len();
    drop(pools);

    Json(json!({
        "status": "healthy",
        "databases_configured": db_count,
        "databases_connected": connected_dbs,
        "endpoints_configured": endpoint_count,
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
    State(state): State<AppState>,
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

    // Atomically swap both config and pools together to avoid inconsistencies.
    // Hold both write locks simultaneously so no request can see mismatched state.
    let old_pools = {
        let mut config = state.config.write().await;
        let mut pools = state.db_pools.write().await;
        let old_pools = std::mem::replace(&mut *pools, new_pools);
        *config = new_config;
        old_pools
    };

    // Drain old pools *after* releasing the locks so in-flight requests on the
    // old pools can finish naturally. sqlx pool close() waits for active
    // connections to be returned, so this is safe.
    tokio::spawn(async move {
        close_pools(&old_pools).await;
        tracing::debug!("Old database pools drained.");
    });

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

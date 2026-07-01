/// Shared handler utilities: auth extraction, DB context, and ID extraction.
///
/// These functions are duplicated across media, file_store, and static_files
/// modules. They are centralized here to maintain a single source of truth.
use std::collections::HashMap;

use crate::config::types::{DatabaseDriver, EndpointConfig};
use crate::error::AppError;
use crate::middleware::auth::extractor::AuthInfo;
use crate::server::state::AppState;

/// Extract authentication info from request.
///
/// # Errors
///
/// Returns `AppError::Auth` if authentication fails.
/// Returns `AppError::Config` if the auth config is missing.
#[allow(clippy::implicit_hasher)]
pub async fn extract_auth_info(
    state: &AppState,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<AuthInfo, AppError> {
    if endpoint.auth == "none" {
        return Ok(AuthInfo::default());
    }
    let auth_config = state.config.read().await.auth.clone();
    crate::middleware::auth::validate::authenticate::<crate::server::state::InMemoryRevocationStore>(
        &endpoint.auth,
        &auth_config,
        headers,
        query_params,
        None,
    )
    .await
}

/// Extract user ID from auth info for ownership checks.
///
/// Returns an empty string when auth is disabled.
pub async fn extract_user_id(
    state: &AppState,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<String, AppError> {
    let auth_info = extract_auth_info(state, endpoint, headers, query_params).await?;
    Ok(auth_info.subject)
}

/// Resolve database pool, table config, and driver for a media config.
pub async fn get_db_context(
    state: &AppState,
    config: &crate::config::types::MediaConfig,
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

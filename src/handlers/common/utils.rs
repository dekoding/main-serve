/// Shared handler utilities: auth extraction, DB context, and ID extraction.
///
/// These functions are duplicated across media, file_store, and static_files
/// modules. They are centralized here to maintain a single source of truth.
use std::collections::HashMap;
use std::path::Path;

use axum::extract::State;

use crate::config::types::{CacheRuleConfig, DEFAULT_CACHE_MAX_AGE, TableConfig};
use crate::config::types::{DatabaseDriver, EndpointConfig, RegisterConfig};
use crate::db::pool::DatabasePool;
use crate::error::AppError;
use crate::middleware::auth::extractor::AuthInfo;
use crate::server::state::AppState;

use axum::http::HeaderValue;
use axum::response::Response;
use http::header;

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

pub struct DatabaseContext {
    pub pool: DatabasePool,
    pub table_config: TableConfig,
    pub driver: DatabaseDriver,
}

/// Get database pool from state
pub async fn get_db_pool(state: &AppState, database: &String) -> Result<DatabasePool, AppError> {
    let pool = {
        let pools = state.db_pools.read().await;
        pools
            .get(database.as_str())
            .ok_or_else(|| AppError::Internal(format!("Database '{}' has no pool", database)))?
            .clone()
    };
    Ok(pool)
}

/// Resolve database pool, table config, and driver.
pub async fn get_db_context(
    state: &AppState,
    database: String,
    table: String,
) -> Result<DatabaseContext, AppError> {
    let pool = get_db_pool(state, &database).await?;
    let driver = pool.driver();

    let table_config = {
        let config_guard = state.config.read().await;
        config_guard
            .tables
            .iter()
            .find(|t| t.name == table && t.database == database)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "Table '{}' in database '{}' not found in config",
                    table, database
                ))
            })?
            .clone()
    };

    Ok(DatabaseContext {
        pool,
        table_config,
        driver,
    })
}

/// Helper function to get the registration database pool and config.
pub async fn get_registration_pool(
    state: State<AppState>,
) -> Result<(DatabasePool, RegisterConfig), AppError> {
    let (pool, register_config) = {
        let register_config =
            {
                let config = state.config.read().await;
                config.auth.register.as_ref().cloned().ok_or_else(|| {
                    AppError::Config("User registration is not enabled".to_string())
                })?
            };

        let pool = get_db_pool(&state, &register_config.database).await?;
        (pool, register_config)
    };
    Ok((pool, register_config))
}

/// Apply Content-Type header to a response.
pub fn apply_content_type(response: &mut Response, content_type: &str) {
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
}

/// Apply Cache-Control header to a response.
pub fn apply_cache_control(
    response: &mut Response,
    cache_max_age: u64,
    path: Option<&Path>,
    cache_rules: &[CacheRuleConfig],
) {
    let cache_control = match (path, cache_rules.is_empty()) {
        (Some(p), false) => get_cache_control(p, cache_max_age, cache_rules),
        _ => format!("public, max-age={cache_max_age}"),
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str(&cache_control).unwrap_or_else(|_| {
            HeaderValue::from_str(&format!("public, max-age={}", DEFAULT_CACHE_MAX_AGE))
                .unwrap_or_else(|_| HeaderValue::from_static("public"))
        }),
    );
}

/// Get the Cache-Control header value for a path.
///
/// Checks extension-specific cache rules first, then falls back to the
/// default max-age.
///
/// Cache rule extensions may include or omit the leading dot (e.g. ".js" or "js").
/// File path extensions from `Path::extension()` do not include the dot.
pub fn get_cache_control(
    path: &Path,
    default_max_age: u64,
    cache_rules: &[CacheRuleConfig],
) -> String {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    for rule in cache_rules {
        if rule.extensions.iter().any(|e| {
            let rule_ext = e.trim_start_matches('.');
            rule_ext == ext
        }) {
            return rule.cache_control.clone();
        }
    }

    format!("public, max-age={default_max_age}")
}

/// Format a `SystemTime` as YYYY-MM-DD HH:MM.
#[must_use]
pub fn format_modified(time: std::time::SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Local> = time.into();
    dt.format("%Y-%m-%d %H:%M").to_string()
}

/// Format a byte count as a human-readable size string.
#[must_use]
pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    for unit in UNITS {
        if size < 1024.0 {
            return if *unit == "B" {
                format!("{size:.0} {unit}")
            } else {
                format!("{size:.1} {unit}")
            };
        }
        size /= 1024.0;
    }
    format!("{size:.1} PiB")
}

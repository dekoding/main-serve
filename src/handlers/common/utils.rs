/// Shared handler utilities: auth extraction, DB context, and ID extraction.
///
/// These functions are duplicated across media, `file_store`, and `static_files`
/// modules. They are centralized here to maintain a single source of truth.
use std::collections::HashMap;
use std::path::Path;

use crate::config::types::EndpointConfig;
use crate::config::types::{CacheRuleConfig, DEFAULT_CACHE_MAX_AGE, TableConfig};
use crate::db::pool::DatabasePool;
use crate::error::AppError;
use crate::middleware::auth::extractor::AuthInfo;
use crate::server::state::AppState;

use axum::http::HeaderValue;
use axum::response::Response;
use http::header;

/// `HandlerContext`
pub struct HandlerContext<'a> {
    /// Application state with access to config and databases.
    pub state: &'a AppState,
    /// Endpoint configuration for this request.
    pub endpoint: &'a EndpointConfig,
    /// HTTP request headers.
    pub headers: &'a axum::http::HeaderMap,
    /// Parsed query parameters.
    pub query_params: &'a HashMap<String, String>,
}

impl HandlerContext<'_> {
    /// Extract authentication info from request.
    ///
    /// # Errors
    ///
    /// Returns `AppError::Auth` if authentication fails.
    /// Returns `AppError::ConfigurationError` if the auth config is missing.
    pub async fn extract_auth_info(&self) -> Result<AuthInfo, AppError> {
        // extract_auth_info(self.state, self.endpoint, self.headers, self.query_params).await
        if self.endpoint.auth == "none" {
            return Ok(AuthInfo::default());
        }
        let auth_config = self.state.config.read().await.auth.clone();
        crate::middleware::auth::validate::authenticate::<
            crate::server::state::InMemoryRevocationStore,
        >(
            &self.endpoint.auth,
            &auth_config,
            self.headers,
            self.query_params,
            None,
        )
        .await
    }

    /// Extract authenticated user ID for ownership checks.
    ///
    /// Returns an empty string when auth is disabled.
    ///
    /// # Errors
    ///
    /// Returns `AppError::Auth` if authentication fails.
    pub async fn extract_user_id(&self) -> Result<String, AppError> {
        Ok(self.extract_auth_info().await?.subject)
    }
}

/// `DatabaseContext`
pub struct DatabaseContext {
    /// Database connection pool.
    pub pool: DatabasePool,
    /// Table schema configuration.
    pub table_config: TableConfig,
}

/// Apply Content-Type header to a response.
pub fn apply_content_type(response: &mut Response, content_type: &str) {
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
}

/// Apply Content-Range header to a response.
pub fn apply_content_range(response: &mut Response, content_range: &str) {
    response.headers_mut().insert(
        header::CONTENT_RANGE,
        HeaderValue::from_str(content_range).unwrap_or(HeaderValue::from_static("bytes */0")),
    );
}

/// Apply Content-Length header to a response.
pub fn apply_content_length(response: &mut Response, content_length: &str) {
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(content_length).unwrap_or(HeaderValue::from_static("0")),
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
            HeaderValue::from_str(&format!("public, max-age={DEFAULT_CACHE_MAX_AGE}"))
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
#[must_use]
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

/// Filter a JSON value body to only include fields that are writable.
///
/// Returns a `serde_json::Map` containing only the key-value pairs from
/// `body` whose keys are present in `writable_columns`.
///
/// # Arguments
///
/// * `body` - The request body as a JSON value.
/// * `writable_columns` - List of column names that are allowed to be written.
///
/// # Examples
///
/// ```
/// extern crate main_serve;
/// use main_serve::handlers::common::utils::filter_writable_body;
///
/// let body = serde_json::json!({"name": "test", "ignored": true});
/// let writable = vec!["name".to_string()];
/// let filtered = filter_writable_body(&body, &writable);
/// assert!(filtered.contains_key("name"));
/// assert!(!filtered.contains_key("ignored"));
/// ```
#[must_use]
pub fn filter_writable_body(
    body: &serde_json::Value,
    writable_columns: &[String],
) -> serde_json::Map<String, serde_json::Value> {
    let mut result = serde_json::Map::new();

    if let Some(obj) = body.as_object() {
        for (k, v) in obj {
            if writable_columns.contains(&k.clone()) {
                result.insert(k.clone(), v.clone());
            }
        }
    }

    result
}

/// Format a `SystemTime` as YYYY-MM-DD HH:MM.
#[must_use]
/// `format_modified`
pub fn format_modified(time: std::time::SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Local> = time.into();
    dt.format("%Y-%m-%d %H:%M").to_string()
}

/// Format a byte count as a human-readable size string.
#[must_use]
/// `format_size`
pub fn format_size(bytes: u64) -> String {
    /// Defines the ordered list of size units for human-readable byte formatting.
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::cast_sign_loss)]
    #[allow(clippy::cast_precision_loss)]
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

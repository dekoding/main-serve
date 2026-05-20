/// Application state: shared config, database pools, and runtime resources.
///
/// `AppState` is the central shared state for all request handlers. It is
/// cheaply cloneable (everything behind `Arc`) and passed to handlers via
/// axum's `State` extractor.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::config::AppConfig;
use crate::config::types::EndpointConfig;
use crate::db::pool::DatabasePool;
use crate::middleware::auth::validators::oauth2::PendingOAuth2;
use crate::middleware::rate_limit::RateLimiter;
use crate::storage::backend::native::NativeStorage;

/// Shared application state available to all handlers.
#[derive(Clone)]
pub struct AppState {
    /// The current parsed configuration, swappable on hot-reload.
    pub config: Arc<RwLock<AppConfig>>,

    /// Path to the YAML configuration file (needed for reload).
    pub config_path: Arc<PathBuf>,

    /// Pre-shared admin token for the reload endpoint.
    pub admin_token: Arc<String>,

    /// Named database connection pools, keyed by database name from config.
    pub db_pools: Arc<RwLock<HashMap<String, DatabasePool>>>,

    /// Global rate limiter shared across all endpoints.
    pub rate_limiter: RateLimiter,

    /// Pending `OAuth2` authorization code flow states (state -> PKCE verifier).
    pub oauth2_pending: Arc<Mutex<HashMap<String, PendingOAuth2>>>,

    /// Endpoint configurations keyed by their normalized path, used for lookups
    /// within request handlers when the path parameter is available.
    /// When multiple endpoints share the same path (with different methods),
    /// they are stored as separate entries with path+method suffixes.
    pub endpoint_configs: Arc<RwLock<HashMap<String, EndpointConfig>>>,

    /// File storage backend for static file serving.
    pub storage: NativeStorage,
}

impl AppState {
    /// Create a new `AppState` from an initial config, its file path, and the admin token.
    #[must_use]
    pub fn new(config: AppConfig, config_path: PathBuf, admin_token: String) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_path: Arc::new(config_path),
            admin_token: Arc::new(admin_token),
            db_pools: Arc::new(RwLock::new(HashMap::new())),
            rate_limiter: RateLimiter::new(),
            oauth2_pending: Arc::new(Mutex::new(HashMap::new())),
            endpoint_configs: Arc::new(RwLock::new(HashMap::new())),
            storage: NativeStorage::new(),
        }
    }

    /// Get the endpoint configuration for a given path, using pattern matching
    /// to handle wildcard patterns like `/app/{*rest}` and path parameters.
    pub async fn get_endpoint_config(&self, path: &str) -> Option<EndpointConfig> {
        let configs = self.endpoint_configs.read().await;

        // First try exact match
        if let Some(endpoint) = configs.get(path) {
            return Some(endpoint.clone());
        }

        // Try prefix matching for paths without wildcards
        let mut current = path;
        while !current.is_empty() {
            if let Some(endpoint) = configs.get(current) {
                return Some(endpoint.clone());
            }
            if let Some(pos) = current.rfind('/') {
                current = &current[..pos];
            } else {
                break;
            }
        }

        // Check for wildcard pattern matches (e.g., /app/{*rest} matches /app/foo/bar)
        for (stored_path, endpoint) in configs.iter() {
            if let Some(pattern_base) = stored_path
                .split_once('{')
                .map(|(base, _)| base.trim_end_matches('/'))
                && path.starts_with(pattern_base)
                && (path.len() == pattern_base.len()
                    || path[pattern_base.len()..].starts_with('/'))
            {
                return Some(endpoint.clone());
            }
        }

        None
    }

    /// Get the endpoint configuration for a given path and method.
    /// This is used by the auth middleware to find the correct endpoint config.
    /// Also handles method-specific keys like `/path#POST`.
    pub async fn get_endpoint_config_for_method(
        &self,
        path: &str,
        method: &axum::http::Method,
    ) -> Option<EndpointConfig> {
        let configs = self.endpoint_configs.read().await;
        let method_str = method.as_str().to_uppercase();

        // First try method-specific key (e.g., /api/articles#POST)
        let method_key = format!("{}#{}", path, method_str);
        if let Some(endpoint) = configs.get(&method_key) {
            return Some(endpoint.clone());
        }

        // Then try exact match on path
        if let Some(endpoint) = configs.get(path)
            && endpoint
                .methods
                .iter()
                .any(|m| matches_http_method(m, method))
        {
            return Some(endpoint.clone());
        }

        // Try prefix matching for paths without wildcards
        let mut current = path;
        while !current.is_empty() {
            if let Some(endpoint) = configs.get(current)
                && endpoint
                    .methods
                    .iter()
                    .any(|m| matches_http_method(m, method))
            {
                return Some(endpoint.clone());
            }
            if let Some(pos) = current.rfind('/') {
                current = &current[..pos];
            } else {
                break;
            }
        }

        // Check for wildcard pattern matches (e.g., /app/{*rest} matches /app/foo/bar)
        // Also handles method-specific keys like /app/{*rest}#GET
        for (stored_path, endpoint) in configs.iter() {
            // Extract the base path (without method suffix)
            let path_without_method = stored_path
                .split_once('#')
                .map(|(base, _)| base)
                .unwrap_or(stored_path);

            if let Some(pattern_base) = path_without_method
                .split_once('{')
                .map(|(base, _)| base.trim_end_matches('/'))
                // Match only if path equals pattern_base exactly, or starts with pattern_base + '/'
                // This prevents /api/authors from matching /api/authors/{id}
                && (path == pattern_base || path.starts_with(&(pattern_base.to_string() + "/")))
                && endpoint
                    .methods
                    .iter()
                    .any(|m| matches_http_method(m, method))
            {
                return Some(endpoint.clone());
            }
        }

        None
    }
}

fn matches_http_method(
    config_method: &crate::config::types::HttpMethod,
    req_method: &axum::http::Method,
) -> bool {
    use axum::http::Method;
    match config_method {
        crate::config::types::HttpMethod::Get => req_method == Method::GET,
        crate::config::types::HttpMethod::Post => req_method == Method::POST,
        crate::config::types::HttpMethod::Put => req_method == Method::PUT,
        crate::config::types::HttpMethod::Patch => req_method == Method::PATCH,
        crate::config::types::HttpMethod::Delete => req_method == Method::DELETE,
        crate::config::types::HttpMethod::Head => req_method == Method::HEAD,
        crate::config::types::HttpMethod::Options => req_method == Method::OPTIONS,
    }
}

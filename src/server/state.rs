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
use crate::server::prefix_match::{find_prefix_match, find_wildcard_match};
use crate::storage::{Storage, get_backend};

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
    pub storage: Arc<dyn Storage>,
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
            storage: Arc::from(
                get_backend("native").expect("native storage backend must be available"),
            ),
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
        if let Some(endpoint) = find_prefix_match(&configs, path, |_| true) {
            return Some(endpoint);
        }

        // Check for wildcard pattern matches (e.g., /app/{*rest} matches /app/foo/bar)
        find_wildcard_match(&configs, path, |_| true)
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

        let method_check =
            |endpoint: &EndpointConfig| endpoint.methods.iter().any(|m| m.matches(method));

        // Then try exact match on path
        if let Some(endpoint) = configs.get(path)
            && method_check(endpoint)
        {
            return Some(endpoint.clone());
        }

        // Try prefix matching for paths without wildcards
        if let Some(endpoint) = find_prefix_match(&configs, path, method_check) {
            return Some(endpoint);
        }

        // Check for wildcard pattern matches (e.g., /app/{*rest} matches /app/foo/bar)
        find_wildcard_match(&configs, path, method_check)
    }
}

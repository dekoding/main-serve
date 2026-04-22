/// Application state: shared config, database pools, and runtime resources.
///
/// `AppState` is the central shared state for all request handlers. It is
/// cheaply cloneable (everything behind `Arc`) and passed to handlers via
/// axum's `State` extractor.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::auth::oauth2::PendingOAuth2;
use crate::config::AppConfig;
use crate::db::pool::DatabasePool;
use crate::middleware::rate_limit::RateLimiter;

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
        }
    }
}

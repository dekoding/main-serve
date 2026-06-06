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
use crate::config::types::StoreConfig;
use crate::db::pool::DatabasePool;
use crate::error::AppError;
use crate::middleware::auth::validators::oauth2::PendingOAuth2;
use crate::middleware::rate_limit::RateLimiter;
use crate::server::prefix_match::{find_prefix_match, find_wildcard_match};
use crate::storage::{Storage, create_store};

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

    /// Named storage backends, keyed by store name from config.
    pub stores: Arc<HashMap<String, Arc<dyn Storage>>>,

    /// Store configurations at creation time, used for change detection during hot reload.
    pub store_configs: Arc<RwLock<HashMap<String, StoreConfig>>>,
}

impl AppState {
    /// Create a new `AppState` from an initial config, its file path, and the admin token.
    ///
    /// # Errors
    ///
    /// Returns `AppError::Config` if any store fails to create from config.
    pub async fn new(
        config: AppConfig,
        config_path: PathBuf,
        admin_token: String,
    ) -> Result<Self, AppError> {
        let stores = build_stores_from_config(&config).await?;
        let store_configs = Arc::new(RwLock::new(
            config
                .stores
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ));

        Ok(Self {
            config: Arc::new(RwLock::new(config)),
            config_path: Arc::new(config_path),
            admin_token: Arc::new(admin_token),
            db_pools: Arc::new(RwLock::new(HashMap::new())),
            rate_limiter: RateLimiter::new(),
            oauth2_pending: Arc::new(Mutex::new(HashMap::new())),
            endpoint_configs: Arc::new(RwLock::new(HashMap::new())),
            stores: Arc::new(stores),
            store_configs,
        })
    }

    /// Get a storage store by name.
    #[must_use]
    pub fn get_store(&self, name: &str) -> Option<Arc<dyn Storage>> {
        self.stores.get(name).cloned()
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
        let method_key = format!("{path}#{method_str}");
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

/// Build named storage stores from config.
///
/// If no stores are defined, creates a default native store named "default"
/// for backward compatibility with existing single-backend setups.
///
/// # Errors
///
/// Returns `AppError::Config` if any store fails to create. Store creation
/// can only fail if validation was bypassed or there is a programming error --
/// validation should have caught all configuration errors before `AppState`
/// construction.
pub async fn build_stores_from_config(
    config: &AppConfig,
) -> Result<HashMap<String, Arc<dyn Storage>>, AppError> {
    let mut stores = HashMap::new();

    if config.stores.is_empty() {
        let default_config = crate::config::types::StoreConfig {
            backend: crate::config::types::StoreBackend::Native,
            root: Some("./".to_string()),
            ..Default::default()
        };
        let store = create_store(&default_config).await.map_err(|e| {
            AppError::Config(format!(
                "Failed to create default native store: {e}. \
                 This should not happen - validation runs before AppState construction"
            ))
        })?;
        stores.insert("default".to_string(), store);
    } else {
        for (name, store_config) in &config.stores {
            let store = create_store(store_config).await.map_err(|e| {
                AppError::Config(format!(
                    "Failed to create storage store '{name}': {e}. \
                     This should not happen - validation runs before AppState construction"
                ))
            })?;
            stores.insert(name.clone(), store);
        }
    }

    Ok(stores)
}

/// Recompute store changes between old and new configurations.
///
/// Returns two vectors:
/// - `unchanged`: Store names that exist in both configs with the same backend+root
/// - `changed_or_removed`: Store names that need recreation (changed or removed from config)
#[must_use]
pub fn compute_store_changes(
    old_configs: &HashMap<String, StoreConfig>,
    new_configs: &HashMap<String, StoreConfig>,
) -> (Vec<String>, Vec<String>) {
    let mut unchanged = Vec::new();
    let mut changed_or_removed = Vec::new();

    // Check all old configs
    for (name, old_cfg) in old_configs {
        if let Some(new_cfg) = new_configs.get(name) {
            if store_config_eq(old_cfg, new_cfg) {
                unchanged.push(name.clone());
            } else {
                // Changed - will be recreated
                changed_or_removed.push(name.clone());
            }
        } else {
            // Removed
            changed_or_removed.push(name.clone());
        }
    }

    // Check for newly added stores
    for name in new_configs.keys() {
        if !old_configs.contains_key(name) {
            changed_or_removed.push(name.clone());
        }
    }

    // Remove duplicates and sort for determinism
    changed_or_removed.sort();
    changed_or_removed.dedup();

    (unchanged, changed_or_removed)
}

/// Compare two store configurations for equality.
///
/// Checks backend type, root, and cloud config identifying fields
/// (bucket, account/container, project). Returns true only when a store
/// needs no recreation.
fn store_config_eq(a: &StoreConfig, b: &StoreConfig) -> bool {
    if a.backend != b.backend {
        return false;
    }
    if a.root != b.root {
        return false;
    }
    match a.backend {
        crate::config::types::StoreBackend::Native | crate::config::types::StoreBackend::Memory => {
            true
        }
        crate::config::types::StoreBackend::S3 => {
            let a_s3 = a.s3.as_ref();
            let b_s3 = b.s3.as_ref();
            match (a_s3, b_s3) {
                (Some(a_s), Some(b_s)) => a_s.bucket == b_s.bucket,
                _ => a_s3.is_none() && b_s3.is_none(),
            }
        }
        crate::config::types::StoreBackend::Azure => {
            let a_azure = a.azure.as_ref();
            let b_azure = b.azure.as_ref();
            match (a_azure, b_azure) {
                (Some(a_a), Some(b_a)) => a_a.container == b_a.container,
                _ => a_azure.is_none() && b_azure.is_none(),
            }
        }
        crate::config::types::StoreBackend::Gcs => {
            let a_gcs = a.gcs.as_ref();
            let b_gcs = b.gcs.as_ref();
            match (a_gcs, b_gcs) {
                (Some(a_g), Some(b_g)) => a_g.bucket == b_g.bucket,
                _ => a_gcs.is_none() && b_gcs.is_none(),
            }
        }
    }
}

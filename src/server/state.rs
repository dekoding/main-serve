/// Application state: shared config, database pools, and runtime resources.
///
/// `AppState` is the central shared state for all request handlers. It is
/// cheaply cloneable (everything behind `Arc`) and passed to handlers via
/// axum's `State` extractor.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use tokio::sync::{Mutex, RwLock};

use crate::config::AppConfig;
use crate::config::types::EndpointConfig;
use crate::config::types::RoleHierarchy;
use crate::config::types::StoreConfig;
use crate::db::pool::DatabasePool;
use crate::db::query::revocation;
use crate::error::AppError;
use crate::middleware::auth::validators::oauth2::PendingOAuth2;
use crate::middleware::rate_limit::RateLimiter;
use crate::server::prefix_match::{find_prefix_match, find_wildcard_match};
use crate::storage::{Storage, create_store};

/// Common trait for token revocation stores.
///
/// Both the in-memory and database-backed revocation stores implement
/// this trait, allowing `AppState` to hold either variant behind a
/// single type.
#[async_trait::async_trait]
pub trait RevocationStoreBackend: Send + Sync {
    /// Check whether the given JTI has been revoked.
    async fn is_revoked(&self, jti: &str) -> bool;

    /// Revoke a token by its JTI. The token remains revoked until the
    /// provided expiry instant.
    async fn revoke(&self, jti: &str, expires_at: Instant);
}

/// In-memory token revocation store.
///
/// Tracks revoked JWTs by their `jti` claim value until their original
/// expiry time. Entries are lazily cleaned up during revocation checks
/// and periodic cleanup runs.
#[derive(Debug, Clone, Default)]
pub struct InMemoryRevocationStore {
    /// Map of JTI -> revocation expiry instant, wrapped in Arc for shared cloning.
    revoked: Arc<tokio::sync::Mutex<std::collections::HashMap<String, Instant>>>,
}

impl InMemoryRevocationStore {
    /// Remove entries whose expiry has passed.
    pub async fn cleanup(&self) {
        let now = Instant::now();
        self.revoked.lock().await.retain(|_, expiry| *expiry > now);
    }
}

#[async_trait::async_trait]
impl RevocationStoreBackend for InMemoryRevocationStore {
    async fn is_revoked(&self, jti: &str) -> bool {
        self.revoked.lock().await.contains_key(jti)
    }

    async fn revoke(&self, jti: &str, expires_at: Instant) {
        self.revoked
            .lock()
            .await
            .insert(jti.to_string(), expires_at);
    }
}

/// Database-backed token revocation store.
///
/// Stores revoked JWTs in a database table (`token_blacklist` by default)
/// with columns: `jti` (varchar primary key), `revoked_at` (timestamptz),
/// `expires_at` (timestamptz). Entries are cleaned up periodically based
/// on the configured interval.
#[derive(Debug, Clone)]
pub struct DatabaseRevocationStore {
    /// The database pool for this store.
    pool: DatabasePool,
    /// Table name for the revocation store (default: "token_blacklist").
    table_name: String,
}

impl DatabaseRevocationStore {
    /// Create a new database-backed revocation store.
    #[must_use]
    pub fn new(pool: DatabasePool, table_name: String) -> Self {
        Self { pool, table_name }
    }

    /// Remove expired revocation entries from the database.
    ///
    /// Deletes rows where `expires_at` is in the past.
    ///
    /// # Errors
    ///
    /// Returns `AppError::Database` if the deletion query fails.
    pub async fn cleanup_expired(&self) -> Result<(), AppError> {
        let built = revocation::build_revoke_cleanup(&self.table_name, self.pool.driver());
        let _ = self.pool.execute_raw(&built.sql).await?;
        Ok(())
    }

    /// Check whether the given JTI has been revoked in the database.
    ///
    /// # Errors
    ///
    /// Returns `AppError::Database` if the lookup query fails.
    pub async fn is_revoked_db(&self, jti: &str) -> bool {
        let built =
            revocation::build_revoke_check(&self.table_name, self.pool.driver(), jti.into());
        let result = self
            .pool
            .fetch_optional_json(&built.sql, &built.params)
            .await;
        matches!(result, Ok(Some(_)))
    }

    /// Revoke a token in the database by its JTI.
    ///
    /// Upserts the revocation entry: if the JTI already exists, updates
    /// `expires_at`; otherwise inserts a new row.
    ///
    /// # Errors
    ///
    /// Returns `AppError::Database` if the insert/update query fails.
    pub async fn revoke_db(&self, jti: &str, expires_at: Instant) -> Result<(), AppError> {
        let now = chrono::Utc::now();
        let expires_at_utc = now + expires_at.saturating_duration_since(Instant::now());
        let revoked_at_str = now.to_rfc3339();
        let expires_at_str = expires_at_utc.to_rfc3339();

        let built = revocation::build_revoke_insert(
            &self.table_name,
            jti,
            &revoked_at_str,
            &expires_at_str,
            self.pool.driver(),
        )?;

        self.pool
            .execute_with_params(&built.sql, &built.params)
            .await?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl RevocationStoreBackend for DatabaseRevocationStore {
    async fn is_revoked(&self, jti: &str) -> bool {
        self.is_revoked_db(jti).await
    }

    async fn revoke(&self, jti: &str, expires_at: Instant) {
        let _ = self.revoke_db(jti, expires_at).await;
    }
}

/// Unified revocation store that can be either in-memory or database-backed.
#[derive(Debug)]
pub enum RevocationStoreImpl {
    /// In-memory store.
    InMemory(Arc<InMemoryRevocationStore>),
    /// Database-backed store.
    Database(Arc<DatabaseRevocationStore>),
}

impl Clone for RevocationStoreImpl {
    fn clone(&self) -> Self {
        match self {
            Self::InMemory(inner) => Self::InMemory(inner.clone()),
            Self::Database(inner) => Self::Database(inner.clone()),
        }
    }
}

#[async_trait::async_trait]
impl RevocationStoreBackend for RevocationStoreImpl {
    async fn is_revoked(&self, jti: &str) -> bool {
        match self {
            Self::InMemory(store) => store.is_revoked(jti).await,
            Self::Database(store) => store.is_revoked(jti).await,
        }
    }

    async fn revoke(&self, jti: &str, expires_at: Instant) {
        match self {
            Self::InMemory(store) => store.revoke(jti, expires_at).await,
            Self::Database(store) => store.revoke(jti, expires_at).await,
        }
    }
}

impl RevocationStoreImpl {
    /// Clean up expired revocation entries for the database store variant.
    ///
    /// This is a no-op for the in-memory variant (which cleans up lazily).
    ///
    /// # Errors
    ///
    /// Returns `AppError::Database` if the cleanup query fails for the
    /// database variant. Returns `Ok(())` for the in-memory variant.
    pub async fn cleanup_expired(&self) -> Result<(), AppError> {
        match self {
            Self::Database(store) => store.cleanup_expired().await,
            Self::InMemory(_) => Ok(()),
        }
    }

    /// Get the cleanup interval in seconds for the database store variant.
    ///
    /// Returns `None` for the in-memory variant.
    #[must_use]
    pub fn cleanup_interval_secs(&self) -> Option<u64> {
        match self {
            Self::Database(_) => None, // Caller passes interval from config
            Self::InMemory(_) => None,
        }
    }
}

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

    /// Precomputed transitive closure of the role hierarchy.
    /// Maps each role to the set of roles it inherits from (excluding itself).
    /// Populated at startup from `config.role_hierarchy`.
    pub role_inheritance: Arc<RwLock<HashMap<String, HashSet<String>>>>,

    /// Token revocation store (in-memory or database-backed). Used when JWT
    /// revocation is enabled. Initialized after database pools are available
    /// (for database-backed stores). Wrapped in OnceLock for deferred init.
    pub revocation_store: OnceLock<RevocationStoreImpl>,
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

        let role_inheritance = compute_role_inheritance(&config.role_hierarchy);
        let role_inheritance = Arc::new(RwLock::new(role_inheritance));

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
            role_inheritance,
            revocation_store: OnceLock::new(),
        })
    }

    /// Set the revocation store after database pools are available.
    ///
    /// This is used by the database-backed revocation store variant, which
    /// requires a database pool at construction time. The pool is created
    /// in `build_app()` after `AppState` is initialized.
    ///
    /// # Errors
    ///
    /// Returns `AppError::Internal` if the revocation store has already been set.
    pub fn set_revocation_store(&self, store: RevocationStoreImpl) -> Result<(), AppError> {
        self.revocation_store
            .set(store)
            .map_err(|_| AppError::Internal("Revocation store already initialized".to_string()))
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

/// Compute the transitive closure of the role hierarchy.
///
/// For each role, determines all roles it inherits from (parents, parents'
/// parents, etc.) and returns a map from role name to the set of inherited
/// roles. Roles not present in the hierarchy map to an empty set.
#[must_use]
pub fn compute_role_inheritance(
    role_hierarchy: &Option<RoleHierarchy>,
) -> HashMap<String, HashSet<String>> {
    let mut closure = HashMap::new();

    let Some(hierarchy) = role_hierarchy else {
        return closure;
    };

    let roles = &hierarchy.roles;

    // Compute transitive closure for each defined role using iterative BFS.
    for role in roles.keys() {
        let mut inherited = HashSet::new();
        let mut stack = roles.get(role).cloned().unwrap_or_default();

        while let Some(parent) = stack.pop() {
            if inherited.insert(parent.clone()) {
                // Add this parent's parents to the stack for transitive resolution.
                if let Some(parent_parents) = roles.get(&parent) {
                    for pp in parent_parents {
                        stack.push(pp.clone());
                    }
                }
            }
        }

        closure.insert(role.clone(), inherited);
    }

    closure
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_revocation_store_is_not_revoked_initially() {
        let store = InMemoryRevocationStore::default();
        assert!(!store.is_revoked("jti-1").await);
    }

    #[tokio::test]
    async fn test_revocation_store_revoke_and_check() {
        let store = InMemoryRevocationStore::default();
        let expires_at = std::time::Instant::now() + std::time::Duration::from_secs(3600);

        store.revoke("jti-1", expires_at).await;
        assert!(store.is_revoked("jti-1").await);
        assert!(!store.is_revoked("jti-2").await);
    }

    #[tokio::test]
    async fn test_revocation_store_cleanup_removes_expired() {
        let store = InMemoryRevocationStore::default();
        let expired = std::time::Instant::now() - std::time::Duration::from_secs(60);
        let valid = std::time::Instant::now() + std::time::Duration::from_secs(3600);

        store.revoke("expired-jti", expired).await;
        store.revoke("valid-jti", valid).await;

        assert!(store.is_revoked("expired-jti").await);
        assert!(store.is_revoked("valid-jti").await);

        store.cleanup().await;

        assert!(!store.is_revoked("expired-jti").await);
        assert!(store.is_revoked("valid-jti").await);
    }

    #[tokio::test]
    async fn test_revocation_store_cleanup_empty() {
        let store = InMemoryRevocationStore::default();
        store.cleanup().await;
        assert!(!store.is_revoked("anything").await);
    }

    #[tokio::test]
    async fn test_revocation_store_multiple_entries() {
        let store = InMemoryRevocationStore::default();
        let expires = std::time::Instant::now() + std::time::Duration::from_secs(3600);

        for i in 0..100 {
            store.revoke(&format!("jti-{i}"), expires).await;
        }

        for i in 0..100 {
            assert!(store.is_revoked(&format!("jti-{i}")).await);
        }
        assert!(!store.is_revoked("nonexistent").await);
    }

    #[tokio::test]
    async fn test_revocation_store_cleanup_all_expired() {
        let store = InMemoryRevocationStore::default();
        let expired = std::time::Instant::now() - std::time::Duration::from_secs(1);

        store.revoke("jti-1", expired).await;
        store.revoke("jti-2", expired).await;

        store.cleanup().await;

        assert!(!store.is_revoked("jti-1").await);
        assert!(!store.is_revoked("jti-2").await);
    }
}

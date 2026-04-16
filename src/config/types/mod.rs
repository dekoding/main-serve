/// Strongly-typed configuration structs deserialized from YAML.
///
/// Every struct implements `Default` with production-safe defaults and uses
/// `#[serde(default)]` so partial YAML configs work out of the box.
///
/// Types are organized into submodules by domain:
/// - `server` - Server bind address, port, TLS, runtime settings
/// - `logging` - Log levels and output formats
/// - `cors` - CORS policy
/// - `rate_limit` - Rate limiting
/// - `database` - Database connections and table schemas
/// - `auth` - Authentication providers (JWT, API key, Basic, OAuth2)
/// - `endpoints` - Endpoint definitions and action configs (CRUD, proxy, static, custom)
mod auth;
mod cors;
mod database;
mod endpoints;
mod logging;
mod rate_limit;
mod server;

use std::collections::HashMap;

use serde::Deserialize;

// Re-export all public types so existing `use crate::config::types::Foo` paths keep working.
pub use auth::*;
pub use cors::*;
pub use database::*;
pub use endpoints::*;
pub use logging::*;
pub use rate_limit::*;
pub use server::*;

// =============================================================================
// Root config
// =============================================================================

/// Top-level configuration - the direct deserialization target for the YAML file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    /// Server bind address, port, TLS, and runtime settings.
    pub server: ServerConfig,
    /// Logging level and output format.
    pub logging: LoggingConfig,
    /// Global CORS policy.
    pub cors: CorsConfig,
    /// Global rate limiting configuration.
    pub rate_limit: RateLimitConfig,
    /// Named database connections.
    #[serde(default)]
    pub databases: HashMap<String, DatabaseConfig>,
    /// Table schemas for auto-migration and query building.
    #[serde(default)]
    pub tables: HashMap<String, TableConfig>,
    /// Authentication provider configurations.
    #[serde(default)]
    pub auth: AuthConfig,
    /// Endpoint definitions.
    #[serde(default)]
    pub endpoints: Vec<EndpointConfig>,
}

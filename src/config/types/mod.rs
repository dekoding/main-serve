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
/// - `auth` - Authentication providers (JWT, API key, Basic, `OAuth2`)
/// - `endpoints` - Endpoint definitions and action configs (CRUD, proxy, static, custom)
/// - `store` - Storage backend configurations (S3, Azure, GCS, native)
/// - `schema` - JSON Schema types for column-level validation
mod auth;
mod cors;
/// Database connection and table schema configuration.
mod database;
/// Endpoint action configurations (CRUD, proxy, static, custom).
mod endpoints;
/// Logging level and output format configuration.
mod logging;
/// Rate limiting configuration.
mod rate_limit;
/// JSON Schema types for column-level validation.
pub(crate) mod schema;
/// Server bind address, port, TLS, and runtime settings.
mod server;
/// Storage backend configurations (S3, Azure, GCS, native).
mod store;

use std::collections::HashMap;

use serde::Deserialize;

// Re-export all public types so existing `use crate::config::types::Foo` paths keep working.
pub use auth::*;
pub use cors::*;
pub use database::*;
pub use endpoints::*;
pub use logging::*;
pub use rate_limit::*;
pub use schema::{GlobalSchema, JsonSchema, SchemaSource};
pub use server::*;
pub use store::*;

// =============================================================================
// Role hierarchy
// =============================================================================

/// Role hierarchy configuration defining inheritance relationships between roles.
///
/// Each key is a role name, and its value is a list of parent roles it inherits
/// from. A role transitively inherits all permissions of its parents, grandparents, etc.
///
/// Example:
/// ```yaml
/// role_hierarchy:
///   admin:
///     - editor
///   editor:
///     - author
/// ```
/// This means `admin` inherits from `editor` (and transitively from `author`),
/// and `editor` inherits from `author`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(transparent)]
/// `RoleHierarchy`
pub struct RoleHierarchy {
    /// Role name -> list of parent roles it inherits from.
    pub roles: HashMap<String, Vec<String>>,
}

// =============================================================================
// Root config
// =============================================================================

/// Top-level configuration - the direct deserialization target for the YAML file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `AppConfig`
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
    pub tables: Vec<TableConfig>,
    /// Authentication provider configurations.
    #[serde(default)]
    pub auth: AuthConfig,
    /// Endpoint definitions.
    #[serde(default)]
    pub endpoints: Vec<EndpointConfig>,
    /// Named storage backends. Endpoints reference these by key.
    #[serde(default)]
    pub stores: HashMap<String, StoreConfig>,
    /// Role inheritance hierarchy. A role inherits all permissions of its
    /// listed parent roles transitively.
    #[serde(default)]
    pub role_hierarchy: Option<RoleHierarchy>,
    /// Named JSON Schema documents for reuse across table columns.
    /// Each key is a unique name referenced via `validation_schema_ref`.
    #[serde(default)]
    pub global_schemas: Option<HashMap<String, serde_yaml::Value>>,
}

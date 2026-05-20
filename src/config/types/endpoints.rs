/// Endpoint definitions and action-specific configurations.
use std::collections::HashMap;

use serde::Deserialize;

use super::cors::CorsConfig;
use super::rate_limit::RateLimitConfig;

/// A single endpoint definition - the core unit of Main Serve's behaviour.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointConfig {
    /// URL path pattern (e.g. `/api/users` or `/api/users/{id}`).
    pub path: String,
    /// HTTP methods this endpoint responds to.
    pub methods: Vec<HttpMethod>,
    /// The type of action to perform.
    pub action: EndpointAction,

    /// CRUD action configuration.
    #[serde(default)]
    pub crud: Option<CrudConfig>,
    /// Proxy action configuration.
    #[serde(default)]
    pub proxy: Option<ProxyConfig>,
    /// Static file serving configuration.
    #[serde(default)]
    pub static_files: Option<StaticFilesConfig>,
    /// Custom/static response configuration.
    #[serde(default)]
    pub custom_response: Option<CustomResponseConfig>,

    /// Authentication type (`"none"`, `"jwt"`, `"api_key"`, `"basic"`, `"oauth2"`).
    #[serde(default = "default_auth_none")]
    pub auth: String,
    /// Roles required to access this endpoint (empty = any authenticated user).
    #[serde(default)]
    pub roles: Vec<String>,
    /// Per-endpoint CORS override.
    #[serde(default)]
    pub cors: Option<CorsConfig>,
    /// Per-endpoint rate limit override.
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,
}

fn default_auth_none() -> String {
    "none".to_string()
}

/// Supported HTTP methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Options,
    Head,
}

/// The type of action an endpoint performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointAction {
    Crud,
    Proxy,
    Static,
    CustomResponse,
}

// =============================================================================
// CRUD config
// =============================================================================

/// CRUD-specific configuration for an endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CrudConfig {
    /// Name of the table to operate on.
    pub table: String,
    /// Database name override (defaults to the table's database).
    pub database: Option<String>,
    /// Fields to include in SELECT queries (`["*"]` for all).
    pub fields: Vec<String>,
    /// Fields allowed in INSERT/UPDATE bodies.
    pub writable_fields: Vec<String>,
    /// Pagination settings.
    pub pagination: PaginationConfig,
    /// Filtering settings.
    pub filtering: FilteringConfig,
    /// Sorting settings.
    pub sorting: SortingConfig,
    /// Optional static WHERE clause appended to all queries.
    pub where_clause: Option<String>,
    /// Join definitions for multi-table queries.
    #[serde(default)]
    pub joins: Vec<JoinConfig>,
    /// Virtual computed fields defined by SQL expressions.
    #[serde(default)]
    pub computed_fields: Vec<ComputedFieldConfig>,
}

impl Default for CrudConfig {
    fn default() -> Self {
        Self {
            table: String::new(),
            database: None,
            fields: vec!["*".to_string()],
            writable_fields: Vec::new(),
            pagination: PaginationConfig::default(),
            filtering: FilteringConfig::default(),
            sorting: SortingConfig::default(),
            where_clause: None,
            joins: Vec::new(),
            computed_fields: Vec::new(),
        }
    }
}

/// Pagination settings for list endpoints.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PaginationConfig {
    /// Whether pagination is active.
    pub enabled: bool,
    /// Default number of records per page.
    pub default_page_size: u64,
    /// Maximum allowed page size.
    pub max_page_size: u64,
}

impl Default for PaginationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_page_size: 20,
            max_page_size: 100,
        }
    }
}

/// Filtering settings for list endpoints.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FilteringConfig {
    /// Whether filtering is active.
    pub enabled: bool,
    /// Fields the client may filter on (`["*"]` for all).
    pub allowed_fields: Vec<String>,
}

impl Default for FilteringConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_fields: vec!["*".to_string()],
        }
    }
}

/// Sorting settings for list endpoints.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SortingConfig {
    /// Whether sorting is active.
    pub enabled: bool,
    /// Default sort field (empty = primary key).
    pub default_field: String,
    /// Default sort direction.
    pub default_order: SortOrder,
    /// Fields the client may sort by (`["*"]` for all).
    pub allowed_fields: Vec<String>,
}

impl Default for SortingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_field: String::new(),
            default_order: SortOrder::Asc,
            allowed_fields: vec!["*".to_string()],
        }
    }
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    Asc,
    Desc,
}

/// Join configuration for CRUD endpoints.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinConfig {
    /// Table to join.
    pub table: String,
    /// Join condition (e.g. `"users.id = posts.user_id"`).
    pub on: String,
    /// SQL join type.
    #[serde(default = "default_join_type")]
    pub join_type: JoinType,
    /// Fields to include from the joined table.
    #[serde(default)]
    pub fields: Vec<String>,
}

fn default_join_type() -> JoinType {
    JoinType::Inner
}

/// SQL join type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JoinType {
    Inner,
    Left,
    Right,
}

/// A computed (virtual) field defined by a SQL expression.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputedFieldConfig {
    /// Alias name for the computed field.
    pub name: String,
    /// SQL expression (e.g. `"COALESCE(first_name, '')"`).
    pub expression: String,
}

// =============================================================================
// Proxy config
// =============================================================================

/// Proxy-specific configuration for an endpoint.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProxyConfig {
    /// Upstream base URL (e.g. `"http://backend:3000"`).
    pub upstream: String,
    /// Optional path rewrite rules.
    pub path_rewrite: Option<PathRewriteConfig>,
    /// Extra headers to add to the proxied request.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Timeout settings.
    pub timeouts: ProxyTimeouts,
    /// Maximum upstream response body size in bytes (0 = unlimited).
    #[serde(default = "default_max_response_size")]
    pub max_response_size: u64,
}

fn default_max_response_size() -> u64 {
    256 * 1024 * 1024 // 256 MiB
}

/// Path rewrite rules for proxied requests.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PathRewriteConfig {
    /// Prefix to strip from the incoming path.
    pub strip_prefix: String,
    /// Prefix to prepend after stripping.
    pub add_prefix: String,
}

/// Timeout settings for proxied requests (all in seconds).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProxyTimeouts {
    /// TCP connect timeout in seconds.
    pub connect: u64,
    /// Response read timeout in seconds.
    pub read: u64,
    /// Total request timeout in seconds.
    pub total: u64,
}

impl Default for ProxyTimeouts {
    fn default() -> Self {
        Self {
            connect: 5,
            read: 30,
            total: 60,
        }
    }
}

// =============================================================================
// Static files config
// =============================================================================

/// Static file serving configuration for an endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StaticFilesConfig {
    /// Root directory to serve files from.
    pub root: String,
    /// Index file name (e.g. `"index.html"`).
    pub index: String,
    /// Whether to generate directory listings.
    pub directory_listing: bool,
    /// Cache-Control max-age in seconds.
    pub cache_max_age: u64,
    /// Whether to serve the index file for unmatched routes (SPA mode).
    pub spa_fallback: bool,

    /// File upload configuration (optional).
    #[serde(default)]
    pub upload: Option<UploadConfig>,
    /// User scoping configuration (optional).
    #[serde(default)]
    pub user_scope: Option<UserScopeConfig>,
    /// Image resize configuration (optional).
    #[serde(default)]
    pub image_resize: Option<ImageResizeConfig>,
    /// Streaming configuration (optional).
    #[serde(default)]
    pub streaming: Option<StreamingConfig>,
}

impl Default for StaticFilesConfig {
    fn default() -> Self {
        Self {
            root: "./public".to_string(),
            index: "index.html".to_string(),
            directory_listing: false,
            cache_max_age: 3600,
            spa_fallback: false,
            upload: None,
            user_scope: None,
            image_resize: None,
            streaming: None,
        }
    }
}

/// File upload configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UploadConfig {
    /// Whether uploads are enabled.
    pub enabled: bool,
    /// Maximum upload size in bytes.
    pub max_size: u64,
    /// Allowed file extensions (empty = all).
    pub allowed_extensions: Vec<String>,
    /// Subdirectory pattern for organizing uploads.
    /// Available placeholders: {user_id}, {year}, {month}, {day}, {uuid}
    pub create_subdirectory: Option<String>,
    /// Role required to upload files (optional).
    pub required_role: Option<String>,
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_size: 10 * 1024 * 1024, // 10 MiB
            allowed_extensions: Vec::new(),
            create_subdirectory: None,
            required_role: None,
        }
    }
}

/// User scoping configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UserScopeConfig {
    /// Enable user-scoped file browsing.
    pub enabled: bool,
    /// Role required to access user-scoped files.
    pub required_role: Option<String>,
    /// Pattern for user-specific directories.
    /// Default: "{user_id}"
    pub directory_pattern: String,
    /// Whether the root directory is exposed when user_scope is enabled.
    pub expose_root: bool,
}

impl Default for UserScopeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            required_role: None,
            directory_pattern: "{user_id}".to_string(),
            expose_root: false,
        }
    }
}

/// Image resize configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ImageResizeConfig {
    /// Whether on-demand resizing is enabled.
    pub enabled: bool,
    /// Maximum dimension for auto-resize.
    pub max_dimension: usize,
    /// Supported formats for conversion.
    pub supported_formats: Vec<String>,
    /// Cache directory for resized images (optional).
    /// Defaults to "cache/resized" under the root directory.
    #[serde(default)]
    pub cache_dir: Option<String>,
}

impl Default for ImageResizeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_dimension: 4096,
            supported_formats: vec![
                "jpg".to_string(),
                "jpeg".to_string(),
                "png".to_string(),
                "webp".to_string(),
            ],
            cache_dir: None,
        }
    }
}

/// Streaming configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StreamingConfig {
    /// Enable chunked streaming for large files.
    pub enabled: bool,
    /// Chunk size in bytes (default: 64 KiB).
    pub buffer_size: usize,
    /// Threshold for enabling streaming (files > this size use streaming).
    pub threshold: u64,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            buffer_size: 65536,     // 64 KiB
            threshold: 1024 * 1024, // 1 MiB
        }
    }
}

// =============================================================================
// Custom response config
// =============================================================================

/// Custom/static response configuration for an endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CustomResponseConfig {
    /// HTTP status code.
    pub status: u16,
    /// Content-Type header value.
    pub content_type: String,
    /// Response body.
    pub body: String,
    /// Additional response headers.
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

impl Default for CustomResponseConfig {
    fn default() -> Self {
        Self {
            status: 200,
            content_type: "application/json".to_string(),
            body: String::new(),
            headers: HashMap::new(),
        }
    }
}

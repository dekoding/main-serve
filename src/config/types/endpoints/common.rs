/// Common endpoint configuration types shared across all action types.
use serde::Deserialize;

use crate::config::types::{CorsConfig, RateLimitConfig};

/// Maximum allowed dimension for image resizing.
pub const IMAGE_MAX_DIMENSION: usize = 4096;

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
    pub crud: Option<super::crud::CrudConfig>,
    /// Proxy action configuration.
    #[serde(default)]
    pub proxy: Option<super::proxy::ProxyConfig>,
    /// Static file serving configuration.
    #[serde(default)]
    pub static_files: Option<super::static_files::StaticFilesConfig>,
    /// SPA hosting configuration.
    #[serde(default)]
    pub spa_host: Option<super::spa_host::SpaHostConfig>,
    /// Media library configuration.
    #[serde(default)]
    pub media: Option<super::media::MediaConfig>,
    /// File store (database-backed file catalog) configuration.
    #[serde(default)]
    pub file_store: Option<super::file_store::FileStoreConfig>,
    /// Custom/static response configuration.
    #[serde(default)]
    pub custom_response: Option<super::custom_response::CustomResponseConfig>,

    /// Authentication type (`"none"`, `"jwt"`, `"api_key"`, `"basic"`, `"oauth2"`).
    #[serde(default = "default_auth_none")]
    pub auth: String,
    /// Roles required to access this endpoint.
    ///
    /// Supports two formats:
    /// - Flat list: `["admin", "editor"]` - same roles for all methods.
    /// - Method-specific: `{ get: ["admin"], post: ["admin"], ... }` -
    ///   different roles per HTTP method.
    ///
    /// Empty roles = any authenticated user.
    #[serde(default)]
    pub roles: RolesConfig,
    /// Per-endpoint CORS override.
    #[serde(default)]
    pub cors: Option<CorsConfig>,
    /// Per-endpoint rate limit override.
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,
}

pub(super) fn default_auth_none() -> String {
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

impl HttpMethod {
    /// Check if this config method matches the given HTTP request method.
    #[must_use]
    pub fn matches(&self, method: &axum::http::Method) -> bool {
        match self {
            HttpMethod::Get => method == axum::http::Method::GET,
            HttpMethod::Post => method == axum::http::Method::POST,
            HttpMethod::Put => method == axum::http::Method::PUT,
            HttpMethod::Patch => method == axum::http::Method::PATCH,
            HttpMethod::Delete => method == axum::http::Method::DELETE,
            HttpMethod::Head => method == axum::http::Method::HEAD,
            HttpMethod::Options => method == axum::http::Method::OPTIONS,
        }
    }

    /// Return the uppercase string representation of this HTTP method.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Head => "HEAD",
            HttpMethod::Options => "OPTIONS",
        }
    }
}

/// Role assignment strategy for an endpoint.
///
/// Supports a flat list of roles (applied to all methods) or
/// method-specific role assignments for fine-grained access control.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum RolesConfig {
    /// Flat list of roles applied to all HTTP methods.
    Flat(Vec<String>),
    /// Method-specific role assignments.
    MethodSpecific(MethodSpecificRoles),
}

impl Default for RolesConfig {
    fn default() -> Self {
        Self::Flat(Vec::new())
    }
}

/// Method-specific role configuration.
///
/// Each HTTP method can have its own list of required roles.
/// Fields default to `None` (empty), which means the fallback
/// `RolesConfig::Flat` is used when present.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MethodSpecificRoles {
    /// Roles required for GET requests.
    #[serde(default, rename = "get")]
    pub get: Option<Vec<String>>,
    /// Roles required for POST requests.
    #[serde(default, rename = "post")]
    pub post: Option<Vec<String>>,
    /// Roles required for PUT requests.
    #[serde(default, rename = "put")]
    pub put: Option<Vec<String>>,
    /// Roles required for PATCH requests.
    #[serde(default, rename = "patch")]
    pub patch: Option<Vec<String>>,
    /// Roles required for DELETE requests.
    #[serde(default, rename = "delete")]
    pub delete: Option<Vec<String>>,
    /// Roles required for HEAD requests.
    #[serde(default, rename = "head")]
    pub head: Option<Vec<String>>,
    /// Roles required for OPTIONS requests.
    #[serde(default, rename = "options")]
    pub options: Option<Vec<String>>,
}

impl RolesConfig {
    /// Resolve the required roles for a specific HTTP method.
    ///
    /// For `MethodSpecific`, returns the method-specific list if set,
    /// otherwise falls back to the `Flat` variant. For `Flat`, always
    /// returns the flat list.
    #[must_use]
    pub fn clone_for_method(&self, method: HttpMethod) -> Vec<String> {
        match self {
            Self::MethodSpecific(ms) => match method {
                HttpMethod::Get => ms.get.clone().unwrap_or_default(),
                HttpMethod::Post => ms.post.clone().unwrap_or_default(),
                HttpMethod::Put => ms.put.clone().unwrap_or_default(),
                HttpMethod::Patch => ms.patch.clone().unwrap_or_default(),
                HttpMethod::Delete => ms.delete.clone().unwrap_or_default(),
                HttpMethod::Head => ms.head.clone().unwrap_or_default(),
                HttpMethod::Options => ms.options.clone().unwrap_or_default(),
            },
            Self::Flat(roles) => roles.clone(),
        }
    }
}

/// The type of action an endpoint performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointAction {
    Crud,
    Proxy,
    StaticFiles,
    SpaHost,
    Media,
    FileStore,
    CustomResponse,
}

// =============================================================================
// Default helper functions used by multiple submodule configs
// =============================================================================

pub(super) fn normalize_extension(ext: &str) -> String {
    ext.strip_prefix('.').unwrap_or(ext).to_string()
}

pub(super) fn deserialize_allowed_extensions<'de, D>(
    deserializer: D,
) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Vec::<String>::deserialize(deserializer)?;
    Ok(raw.into_iter().map(|s| normalize_extension(&s)).collect())
}

pub(super) fn default_true() -> bool {
    true
}

pub(super) fn default_true_bool() -> bool {
    true
}

pub(super) fn default_index() -> String {
    "index.html".to_string()
}

pub(super) fn default_cache_max_age() -> u64 {
    3600
}

pub(super) fn default_fallback_status() -> u16 {
    200
}

pub(super) fn default_max_response_size() -> u64 {
    256 * 1024 * 1024 // 256 MiB
}

pub(super) fn default_media_max_size() -> u64 {
    100 * 1024 * 1024 // 100 MiB
}

pub(super) fn default_max_versions() -> u32 {
    10
}

pub(super) fn default_quality() -> u8 {
    85
}

pub(super) fn default_cover_fit() -> super::static_files::ImageResizeFit {
    super::static_files::ImageResizeFit::Cover
}

pub(super) fn default_media_refs_table() -> String {
    "media_entity_refs".to_string()
}

pub(super) fn default_media_id_col() -> String {
    "media_id".to_string()
}

pub(super) fn default_entity_id_col() -> String {
    "entity_id".to_string()
}

pub(super) fn default_content_type_col() -> String {
    "content_type".to_string()
}

pub(super) fn default_order_col() -> String {
    "attachment_order".to_string()
}

pub(super) fn default_media_trash_retention() -> u32 {
    14
}

pub(super) fn default_trash_prefix() -> String {
    ".trash".to_string()
}

pub(super) fn default_trash_admin_roles() -> Vec<String> {
    vec!["admin".to_string()]
}

pub(super) fn default_trash_retention() -> u32 {
    30
}

pub(super) fn default_share_ttl() -> u64 {
    86400
}

pub(super) fn default_share_max_ttl() -> u64 {
    604_800
}

pub(super) fn default_shared_prefix() -> String {
    "shared".to_string()
}

pub(super) fn default_resize_dimension() -> usize {
    IMAGE_MAX_DIMENSION
}

pub(super) fn default_resize_formats() -> Vec<String> {
    vec![
        "jpg".to_string(),
        "jpeg".to_string(),
        "png".to_string(),
        "webp".to_string(),
    ]
}

pub(super) fn default_media_columns() -> Vec<String> {
    vec!["auto".to_string()]
}

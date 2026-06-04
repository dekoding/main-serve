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
    /// SPA hosting configuration.
    #[serde(default)]
    pub spa_host: Option<SpaHostConfig>,
    /// Media library configuration.
    #[serde(default)]
    pub media: Option<MediaConfig>,
    /// File store (database-backed file catalog) configuration.
    #[serde(default)]
    pub file_store: Option<FileStoreConfig>,
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
// CRUD config
// =============================================================================

/// CRUD-specific configuration for an endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CrudConfig {
    /// Name of the table to operate on.
    pub table: String,
    /// Named database this table belongs to (must match a key in `databases`).
    pub database: String,
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
            database: String::new(),
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

/// Per-extension Cache-Control rule.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheRuleConfig {
    /// File extensions to match (e.g. [".html", ".js"]).
    pub extensions: Vec<String>,
    /// Cache-Control header value (e.g. "public, max-age=31536000, immutable").
    pub cache_control: String,
}

/// Static file serving configuration for an endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StaticFilesConfig {
    /// Named store to use (must match a key in `stores`).
    pub storage: String,
    /// Index file name (e.g. "index.html").
    #[serde(default = "default_index")]
    pub index: String,
    /// Whether to generate directory listings.
    #[serde(default)]
    pub directory_listing: bool,
    /// Cache-Control max-age in seconds.
    #[serde(default = "default_cache_max_age")]
    pub cache_max_age: u64,
    /// ETag generation for cache validation.
    #[serde(default = "default_true")]
    pub etag: bool,
    /// Support for HTTP range requests (partial content / 206).
    #[serde(default = "default_true")]
    pub range_requests: bool,
    /// Support for HTTP HEAD method.
    #[serde(default = "default_true")]
    pub head_support: bool,
    /// Per-extension Cache-Control rules.
    #[serde(default)]
    pub cache_rules: Vec<CacheRuleConfig>,
    /// File upload configuration (optional).
    #[serde(default)]
    pub upload: Option<UploadConfig>,
    /// Image resize configuration (optional).
    #[serde(default)]
    pub image_resize: Option<ImageResizeConfig>,
    /// Streaming configuration (optional).
    #[serde(default)]
    pub streaming: Option<StreamingConfig>,
}

fn default_index() -> String {
    "index.html".to_string()
}

fn default_cache_max_age() -> u64 {
    3600
}

impl Default for StaticFilesConfig {
    fn default() -> Self {
        Self {
            storage: String::new(),
            index: "index.html".to_string(),
            directory_listing: false,
            cache_max_age: 3600,
            etag: true,
            range_requests: true,
            head_support: true,
            cache_rules: Vec::new(),
            upload: None,
            image_resize: None,
            streaming: None,
        }
    }
}

/// Normalize allowed extensions by stripping leading dots.
///
/// Handles compound extensions like `.tar.gz` correctly by only
/// removing the leading dot, preserving internal dots.
fn normalize_extension(ext: &str) -> String {
    ext.strip_prefix('.').unwrap_or(ext).to_string()
}

fn deserialize_allowed_extensions<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Vec::<String>::deserialize(deserializer)?;
    Ok(raw.into_iter().map(|s| normalize_extension(&s)).collect())
}

/// MIME type detection method for uploads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UploadMimeDetection {
    Extension,
    #[default]
    Magic,
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
    /// Leading dots are stripped during parsing, so both `.jpg` and `jpg` work.
    #[serde(deserialize_with = "deserialize_allowed_extensions")]
    pub allowed_extensions: Vec<String>,
    /// Subdirectory pattern for organizing uploads.
    /// Available placeholders: {user_id}, {year}, {month}, {day}, {uuid}
    pub create_subdirectory: Option<String>,
    /// MIME type detection method.
    #[serde(default)]
    pub mime_detection: UploadMimeDetection,
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_size: 10 * 1024 * 1024, // 10 MiB
            allowed_extensions: Vec::new(),
            create_subdirectory: None,
            mime_detection: UploadMimeDetection::Magic,
        }
    }
}

/// Image resize fit mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageResizeFit {
    #[default]
    ScaleDown,
    Cover,
    Contain,
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
    /// Default resize fit mode.
    #[serde(default)]
    pub default_fit: ImageResizeFit,
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
            default_fit: ImageResizeFit::ScaleDown,
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
    /// Whether to include Content-Length header in streaming responses.
    #[serde(default = "default_true")]
    pub include_content_length: bool,
}

fn default_true() -> bool {
    true
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            buffer_size: 65536,     // 64 KiB
            threshold: 1024 * 1024, // 1 MiB
            include_content_length: true,
        }
    }
}

// =============================================================================
// Media / File Store config
// =============================================================================

/// Default value for boolean fields that default to `true`.
fn default_true_bool() -> bool {
    true
}

/// Default retention days for trashed file_store entries.
fn default_trash_retention() -> u32 {
    30
}

/// A metadata column definition for media/file_store endpoints.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaMetadataColumn {
    /// Column name.
    pub name: String,
    /// SQL data type (e.g. "text", "bigint", "jsonb").
    #[serde(rename = "type")]
    pub column_type: String,
    /// Whether this column allows NULL values.
    #[serde(default = "default_true_bool")]
    pub nullable: bool,
    /// Default value expression (raw SQL).
    #[serde(default)]
    pub default: Option<String>,
}

/// A metadata column definition for file_store endpoints.
/// Same structure as MediaMetadataColumn.
pub type FileStoreMetadataColumn = MediaMetadataColumn;

/// Field-level permissions for file_store endpoints.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileStoreFieldPermissions {
    /// Roles allowed to read this field. Use ["*"] for all roles.
    pub read: Vec<String>,
    /// Roles allowed to write this field.
    pub write: Vec<String>,
}

/// Ownership configuration for file_store endpoints.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileStoreOwnershipConfig {
    /// Column name that stores the owner ID.
    pub owner_column: String,
    /// Admin users can access any row.
    #[serde(default = "default_true_bool")]
    pub admin_override: bool,
}

/// Trash configuration for file_store endpoints.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileStoreTrashConfig {
    /// Whether trash is enabled.
    #[serde(default = "default_true_bool")]
    pub enabled: bool,
    /// Number of days to retain trashed entries.
    #[serde(default = "default_trash_retention")]
    pub retention_days: u32,
}

// =============================================================================
// Media endpoint config types
// =============================================================================

/// MIME type detection for media uploads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaMimeDetection {
    Extension,
    #[default]
    Magic,
}

/// User scoping mode for media endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaScopeMode {
    User,
    #[default]
    Shared,
    Open,
}

/// On-delete behavior for media content references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaOnDeleteBehavior {
    #[default]
    Detach,
    Cascade,
    Error,
}

/// Facet type for media search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaFacetType {
    Term,
    DateRange,
    Numeric,
}

/// Per-facet configuration for media search.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaFacetConfig {
    /// Facet display name.
    pub name: String,
    /// Database column to facet on.
    pub field: String,
    /// Facet type.
    pub facet_type: MediaFacetType,
}

/// Named image resize style for media (e.g. thumbnail, gallery).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaImageResizeStyle {
    /// Style name (e.g. "thumbnail", "gallery").
    pub name: String,
    /// Maximum width in pixels.
    pub max_width: u32,
    /// Maximum height in pixels.
    pub max_height: u32,
    /// Resize fit mode.
    #[serde(default = "default_cover_fit")]
    pub resize_fit: ImageResizeFit,
    /// Output format (e.g. "webp", "jpg").
    pub format: String,
    /// JPEG/WebP quality (0-100).
    #[serde(default = "default_quality")]
    pub quality: u8,
}

fn default_cover_fit() -> ImageResizeFit {
    ImageResizeFit::Cover
}

fn default_quality() -> u8 {
    85
}

/// Preview configuration for non-image files.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaPreviewConfig {
    /// Preview style name.
    pub name: String,
    /// Maximum preview width.
    pub max_width: u32,
    /// Maximum preview height.
    pub max_height: u32,
    /// Output format.
    pub format: String,
}

/// File versioning configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaVersioningConfig {
    /// Whether versioning is enabled.
    pub enabled: bool,
    /// Maximum number of versions per file.
    #[serde(default = "default_max_versions")]
    pub max_versions: u32,
}

fn default_max_versions() -> u32 {
    10
}

/// Preview generation configuration for non-image files.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaPreviewGenerationConfig {
    /// Whether preview generation is enabled.
    pub enabled: bool,
    /// Preview styles to generate.
    pub previews: Vec<MediaPreviewConfig>,
}

/// Media upload configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaUploadConfig {
    /// Maximum upload size in bytes.
    #[serde(default = "default_media_max_size")]
    pub max_size: u64,
    /// Allowed file extensions.
    #[serde(default)]
    pub allowed_extensions: Vec<String>,
    /// Allowed MIME types.
    #[serde(default)]
    pub allowed_mime_types: Vec<String>,
    /// MIME type detection method.
    #[serde(default)]
    pub mime_detection: MediaMimeDetection,
    /// Whether bulk upload is supported.
    #[serde(default = "default_true_bool")]
    pub bulk_supported: bool,
    /// Subdirectory pattern.
    #[serde(default)]
    pub create_subdirectory: Option<String>,
    /// File versioning settings.
    #[serde(default)]
    pub versioning: Option<MediaVersioningConfig>,
    /// Preview generation settings.
    #[serde(default)]
    pub preview_generation: Option<MediaPreviewGenerationConfig>,
}

fn default_media_max_size() -> u64 {
    100 * 1024 * 1024 // 100 MiB
}

/// Move configuration for media files.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaMoveConfig {
    /// Whether move is enabled.
    #[serde(default = "default_true_bool")]
    pub enabled: bool,
    /// Admin users can move files regardless of scope restrictions.
    #[serde(default = "default_true_bool")]
    pub admin_override: bool,
    /// Automatically create the destination directory structure.
    #[serde(default)]
    pub auto_create_destination: bool,
}

/// Rename configuration for media files.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaRenameConfig {
    /// Whether rename is enabled.
    #[serde(default = "default_true_bool")]
    pub enabled: bool,
    /// Admin users can rename files regardless of scope restrictions.
    #[serde(default = "default_true_bool")]
    pub admin_override: bool,
}

/// Delete configuration for media files.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaDeleteConfig {
    /// Whether delete is enabled.
    #[serde(default = "default_true_bool")]
    pub enabled: bool,
    /// Admin users can delete files regardless of scope restrictions.
    #[serde(default = "default_true_bool")]
    pub admin_override: bool,
}

/// Attach to content configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaAttachConfig {
    /// Whether attaching media to content is enabled.
    #[serde(default = "default_true_bool")]
    pub enabled: bool,
}

/// Detach from content configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaDetachConfig {
    /// Whether detaching media from content is enabled.
    #[serde(default = "default_true_bool")]
    pub enabled: bool,
}

/// User scope configuration for media.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaUserScopeConfig {
    /// Whether user scoping is enabled.
    #[serde(default = "default_true_bool")]
    pub enabled: bool,
    /// User scope mode for media access.
    #[serde(default)]
    pub mode: MediaScopeMode,
    /// Roles that can browse all media regardless of scope.
    #[serde(default)]
    pub admin_roles: Vec<String>,
    /// Allow users to browse media from other users.
    #[serde(default)]
    pub allow_cross_user_browse: bool,
}

/// Content references configuration for media.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaContentReferencesConfig {
    /// Whether content references are enabled.
    #[serde(default = "default_true_bool")]
    pub enabled: bool,
    /// Database table storing media-to-entity references.
    #[serde(default = "default_media_refs_table")]
    pub table: String,
    /// Column name for the media ID foreign key.
    #[serde(default = "default_media_id_col")]
    pub media_id_column: String,
    /// Column name for the entity ID foreign key.
    #[serde(default = "default_entity_id_col")]
    pub entity_id_column: String,
    /// Column name for the content type (e.g. "post", "product").
    #[serde(default = "default_content_type_col")]
    pub content_type_column: String,
    /// Column name for the attachment order/sort value.
    #[serde(default = "default_order_col")]
    pub order_column: String,
    /// Allowed content types (None = all).
    #[serde(default)]
    pub allowed_content_types: Option<Vec<String>>,
    /// Behavior when the referenced entity is deleted.
    #[serde(default)]
    pub on_delete: MediaOnDeleteBehavior,
}

fn default_media_refs_table() -> String {
    "media_entity_refs".to_string()
}

fn default_media_id_col() -> String {
    "media_id".to_string()
}

fn default_entity_id_col() -> String {
    "entity_id".to_string()
}

fn default_content_type_col() -> String {
    "content_type".to_string()
}

fn default_order_col() -> String {
    "attachment_order".to_string()
}

/// Trash configuration for media.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaTrashConfig {
    /// Whether trash is enabled.
    #[serde(default = "default_true_bool")]
    pub enabled: bool,
    /// Number of days to retain trashed media before permanent deletion.
    #[serde(default = "default_media_trash_retention")]
    pub retention_days: u32,
    /// Storage prefix/path for trashed files.
    #[serde(default = "default_trash_prefix")]
    pub prefix: String,
    /// Whether trash management endpoints are exposed.
    #[serde(default = "default_true_bool")]
    pub management_endpoints: bool,
    /// Roles permitted to manage trash.
    #[serde(default = "default_trash_admin_roles")]
    pub admin_roles: Vec<String>,
}

fn default_media_trash_retention() -> u32 {
    14
}

fn default_trash_prefix() -> String {
    ".trash".to_string()
}

fn default_trash_admin_roles() -> Vec<String> {
    vec!["admin".to_string()]
}

/// Sharing configuration for media.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaSharingConfig {
    /// Whether sharing is enabled.
    #[serde(default = "default_true_bool")]
    pub enabled: bool,
    /// Secret key for signing share links.
    pub signing_secret: String,
    /// Default time-to-live for share links in seconds.
    #[serde(default = "default_share_ttl")]
    pub default_ttl: u64,
    /// Maximum time-to-live for share links in seconds.
    #[serde(default = "default_share_max_ttl")]
    pub max_ttl: u64,
    /// Storage prefix/path for shared files.
    #[serde(default = "default_shared_prefix")]
    pub prefix: String,
    /// Whether any authenticated user can share media.
    #[serde(default = "default_true_bool")]
    pub allow_any_user: bool,
    /// Required role to create share links.
    #[serde(default)]
    pub required_link_role: Option<String>,
}

fn default_share_ttl() -> u64 {
    86400
}

fn default_share_max_ttl() -> u64 {
    604800
}

fn default_shared_prefix() -> String {
    "shared".to_string()
}

/// Image resize configuration for media.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaImageResizeConfig {
    /// Whether on-demand image resizing is enabled.
    #[serde(default = "default_true_bool")]
    pub enabled: bool,
    /// Maximum dimension for auto-resize in pixels.
    #[serde(default = "default_resize_dimension")]
    pub max_dimension: usize,
    /// Supported output formats for conversion.
    #[serde(default = "default_resize_formats")]
    pub supported_formats: Vec<String>,
    /// Default resize fit mode.
    #[serde(default)]
    pub default_fit: ImageResizeFit,
    /// Cache directory for resized images.
    #[serde(default)]
    pub cache_dir: Option<String>,
    /// Named image resize styles.
    #[serde(default)]
    pub styles: Vec<MediaImageResizeStyle>,
    /// Whether to generate resized images on upload.
    #[serde(default)]
    pub generate_on_upload: bool,
}

fn default_resize_dimension() -> usize {
    4096
}

fn default_resize_formats() -> Vec<String> {
    vec![
        "jpg".to_string(),
        "jpeg".to_string(),
        "png".to_string(),
        "webp".to_string(),
    ]
}

/// Media library endpoint configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaConfig {
    /// Named store to use.
    pub storage: String,
    /// Database table for media metadata.
    pub table: String,
    /// Named database to use.
    pub database: String,
    /// Metadata presets: "auto", "tags", "description", "alt_text", "content_type".
    #[serde(default = "default_media_columns")]
    pub columns: Vec<String>,
    /// Additional metadata columns not covered by presets.
    #[serde(default)]
    pub metadata_columns: Vec<MediaMetadataColumn>,
    /// Upload configuration.
    #[serde(default)]
    pub upload: Option<MediaUploadConfig>,
    /// Move configuration.
    #[serde(default)]
    #[serde(alias = "move")]
    pub move_config: Option<MediaMoveConfig>,
    /// Rename configuration.
    #[serde(default)]
    pub rename: Option<MediaRenameConfig>,
    /// Delete configuration.
    #[serde(default)]
    pub delete: Option<MediaDeleteConfig>,
    /// Attach to content configuration.
    #[serde(default)]
    pub attach_to_content: Option<MediaAttachConfig>,
    /// Detach from content configuration.
    #[serde(default)]
    pub detach_from_content: Option<MediaDetachConfig>,
    /// User scoping configuration.
    #[serde(default)]
    pub user_scope: Option<MediaUserScopeConfig>,
    /// Content references configuration.
    #[serde(default)]
    pub content_references: Option<MediaContentReferencesConfig>,
    /// Trash configuration.
    #[serde(default)]
    pub trash: Option<MediaTrashConfig>,
    /// Sharing configuration.
    #[serde(default)]
    pub sharing: Option<MediaSharingConfig>,
    /// Pagination for metadata listing.
    #[serde(default)]
    pub pagination: PaginationConfig,
    /// Sorting for metadata listing.
    #[serde(default)]
    pub sorting: SortingConfig,
    /// Filtering for metadata listing.
    #[serde(default)]
    pub filtering: FilteringConfig,
    /// Faceted search configuration.
    #[serde(default)]
    pub facets: Vec<MediaFacetConfig>,
    /// Image resize configuration.
    #[serde(default)]
    pub image_resize: Option<MediaImageResizeConfig>,
}

fn default_media_columns() -> Vec<String> {
    vec!["auto".to_string()]
}

// =============================================================================
// SPA host config
// =============================================================================

/// SPA hosting endpoint configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpaHostConfig {
    /// Named store to use (must match a key in `stores`).
    pub storage: String,
    /// Index file for SPA fallback.
    #[serde(default = "default_index")]
    pub index: String,
    /// Default Cache-Control max-age in seconds.
    #[serde(default = "default_cache_max_age")]
    pub cache_max_age: u64,
    /// Per-extension Cache-Control rules.
    #[serde(default)]
    pub cache_rules: Vec<CacheRuleConfig>,
    /// ETag generation for cache validation.
    #[serde(default = "default_true")]
    pub etag: bool,
    /// HTTP status code for SPA fallback responses (non-existent paths).
    #[serde(default = "default_fallback_status")]
    pub fallback_status: u16,
}

impl Default for SpaHostConfig {
    fn default() -> Self {
        Self {
            storage: String::new(),
            index: "index.html".to_string(),
            cache_max_age: 3600,
            cache_rules: Vec::new(),
            etag: true,
            fallback_status: 200,
        }
    }
}

fn default_fallback_status() -> u16 {
    200
}

// =============================================================================
// File store config
// =============================================================================

/// File store (database-backed file catalog) endpoint configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileStoreConfig {
    /// Named store to use.
    pub storage: String,
    /// Database table that serves as the file registry.
    pub table: String,
    /// Named database to use.
    pub database: String,
    /// File metadata columns tracked by the registry.
    #[serde(default)]
    pub metadata_columns: Vec<FileStoreMetadataColumn>,
    /// Field-level read/write permissions per role.
    #[serde(default)]
    pub field_permissions: Option<HashMap<String, FileStoreFieldPermissions>>,
    /// Row-level authorization via owner column.
    #[serde(default)]
    pub ownership: Option<FileStoreOwnershipConfig>,
    /// Trash support for deleted entries.
    #[serde(default)]
    pub trash: Option<FileStoreTrashConfig>,
    /// Pagination for listing files.
    #[serde(default)]
    pub pagination: PaginationConfig,
    /// Sorting for listing files.
    #[serde(default)]
    pub sorting: SortingConfig,
    /// Filtering for listing files.
    #[serde(default)]
    pub filtering: FilteringConfig,
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

/// Media library endpoint configuration types.
use serde::Deserialize;

use super::common::{
    default_content_type_col, default_cover_fit, default_entity_id_col, default_max_versions,
    default_media_columns, default_media_id_col, default_media_max_size, default_media_refs_table,
    default_media_trash_retention, default_order_col, default_quality, default_resize_dimension,
    default_resize_formats, default_share_max_ttl, default_share_ttl, default_shared_prefix,
    default_trash_admin_roles, default_trash_prefix, default_true_bool,
};

/// MIME type detection for media uploads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
/// `MediaMimeDetection`
pub enum MediaMimeDetection {
    /// Detect MIME type from file extension.
    Extension,
    /// Detect MIME type by inspecting magic bytes.
    #[default]
    Magic,
}

/// User scoping mode for media endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
/// `MediaScopeMode`
pub enum MediaScopeMode {
    /// Each user can only see their own media.
    User,
    /// Media is shared across all authenticated users.
    #[default]
    Shared,
    /// Media is accessible to anyone without authentication.
    Open,
}

/// On-delete behavior for media content references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
/// `MediaOnDeleteBehavior`
pub enum MediaOnDeleteBehavior {
    /// Detach the media reference from the entity on delete.
    #[default]
    Detach,
    /// Cascade-delete the media when the entity is deleted.
    Cascade,
    /// Return an error if the entity has active media references.
    Error,
}

/// Facet type for media search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
/// `MediaFacetType`
pub enum MediaFacetType {
    /// Filter by exact term match.
    Term,
    /// Filter by a date range.
    DateRange,
    /// Filter by a numeric range.
    Numeric,
}

/// A metadata column definition for `media/file_store` endpoints.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
/// `MediaMetadataColumn`
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

/// Per-facet configuration for media search.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
/// `MediaFacetConfig`
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
/// `MediaImageResizeStyle`
pub struct MediaImageResizeStyle {
    /// Style name (e.g. "thumbnail", "gallery").
    pub name: String,
    /// Maximum width in pixels.
    pub max_width: u32,
    /// Maximum height in pixels.
    pub max_height: u32,
    /// Resize fit mode.
    #[serde(default = "default_cover_fit")]
    pub resize_fit: super::static_files::ImageResizeFit,
    /// Output format (e.g. "webp", "jpg").
    pub format: String,
    /// JPEG/WebP quality (0-100).
    #[serde(default = "default_quality")]
    pub quality: u8,
}

/// Preview configuration for non-image files.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `MediaPreviewConfig`
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
/// `MediaVersioningConfig`
pub struct MediaVersioningConfig {
    /// Whether versioning is enabled.
    pub enabled: bool,
    /// Maximum number of versions per file.
    #[serde(default = "default_max_versions")]
    pub max_versions: u32,
}

/// Preview generation configuration for non-image files.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `MediaPreviewGenerationConfig`
pub struct MediaPreviewGenerationConfig {
    /// Whether preview generation is enabled.
    pub enabled: bool,
    /// Preview styles to generate.
    pub previews: Vec<MediaPreviewConfig>,
}

/// Media upload configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `MediaUploadConfig`
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

/// Move configuration for media files.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `MediaMoveConfig`
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
/// `MediaRenameConfig`
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
/// `MediaDeleteConfig`
pub struct MediaDeleteConfig {
    /// Whether delete is enabled.
    #[serde(default = "default_true_bool")]
    pub enabled: bool,
    /// Admin users can delete files regardless of scope restrictions.
    #[serde(default = "default_true_bool")]
    pub admin_override: bool,
}

/// User scope configuration for media.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `MediaUserScopeConfig`
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
/// `MediaContentReferencesConfig`
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

/// Trash configuration for media.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `MediaTrashConfig`
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

/// Sharing configuration for media.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `MediaSharingConfig`
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

/// Image resize configuration for media.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `MediaImageResizeConfig`
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
    pub default_fit: super::static_files::ImageResizeFit,
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

/// Media library endpoint configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `MediaConfig`
pub struct MediaConfig {
    /// Named store to use.
    pub storage: String,
    /// Database table for media metadata.
    pub table: String,
    /// Named database to use.
    pub database: String,
    /// Metadata presets: "auto", "tags", "description", "`alt_text`", "`content_type`".
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
    pub pagination: super::listing::PaginationConfig,
    /// Sorting for metadata listing.
    #[serde(default)]
    pub sorting: super::listing::SortingConfig,
    /// Filtering for metadata listing.
    #[serde(default)]
    pub filtering: super::listing::FilteringConfig,
    /// Faceted search configuration.
    #[serde(default)]
    pub facets: Vec<MediaFacetConfig>,
    /// Image resize configuration.
    #[serde(default)]
    pub image_resize: Option<MediaImageResizeConfig>,
}

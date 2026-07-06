/// File store (database-backed file catalog) configuration types.
use std::collections::HashMap;

use serde::Deserialize;

use super::common::{default_trash_retention, default_true_bool};
use super::media::MediaMetadataColumn;

/// A metadata column definition for `file_store` endpoints.
/// Same structure as `MediaMetadataColumn`.
pub type FileStoreMetadataColumn = MediaMetadataColumn;

/// Field-level permissions for `file_store` endpoints.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileStoreFieldPermissions {
    /// Roles allowed to read this field. Use ["*"] for all roles.
    pub read: Vec<String>,
    /// Roles allowed to write this field.
    pub write: Vec<String>,
}

/// Ownership configuration for `file_store` endpoints.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileStoreOwnershipConfig {
    /// Column name that stores the owner ID.
    pub owner_column: String,
    /// Admin users can access any row.
    #[serde(default = "default_true_bool")]
    pub admin_override: bool,
}

/// Trash configuration for `file_store` endpoints.
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
    pub pagination: super::listing::PaginationConfig,
    /// Sorting for listing files.
    #[serde(default)]
    pub sorting: super::listing::SortingConfig,
    /// Filtering for listing files.
    #[serde(default)]
    pub filtering: super::listing::FilteringConfig,
}

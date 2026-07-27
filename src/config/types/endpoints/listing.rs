use serde::Deserialize;

/// Pagination settings for list endpoints.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `PaginationConfig`
pub struct PaginationConfig {
    /// Whether pagination is active.
    pub enabled: bool,
    /// Default number of records per page.
    pub default_page_size: u64,
    /// Maximum allowed page size.
    pub max_page_size: u64,
}

impl Default for PaginationConfig {
    /// Returns a pagination configuration with pagination enabled and default page size of 20.
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
/// `FilteringConfig`
pub struct FilteringConfig {
    /// Whether filtering is active.
    pub enabled: bool,
    /// Fields the client may filter on (`["*"]` for all).
    pub allowed_fields: Vec<String>,
}

impl Default for FilteringConfig {
    /// Returns a filtering configuration with filtering enabled and all fields allowed.
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
/// `SortingConfig`
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
    /// Returns a sorting configuration with sorting enabled, ascending order, and all fields allowed.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
/// `SortOrder`
pub enum SortOrder {
    /// Ascending order (A-Z, 0-9).
    #[default]
    Asc,
    /// Descending order (Z-A, 9-0).
    Desc,
}

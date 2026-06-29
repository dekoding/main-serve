/// CRUD-specific endpoint configuration types.
use serde::Deserialize;

use crate::db::query::types::JoinConfig;

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
    /// Additional WHERE clause appended to DELETE queries, interpolated with request context (e.g. `"author_id = ${request.user.id}"`).
    pub delete_where_clause: Option<String>,
    /// Additional WHERE clause appended to UPDATE queries, interpolated with request context (e.g. `"author_id = ${request.user.id}"`).
    pub update_where_clause: Option<String>,
    /// Join definitions for multi-table queries.
    #[serde(default)]
    pub joins: Vec<JoinConfig>,
    /// Virtual computed fields defined by SQL expressions.
    #[serde(default)]
    pub computed_fields: Vec<ComputedFieldConfig>,

    /// Name of the column to auto-populate with the authenticated user's ID
    /// during INSERT operations. Supports request context interpolation
    /// (e.g. `"${request.user.id}"`). If the request body already contains
    /// this field, the body value takes precedence.
    #[serde(default)]
    pub insert_owner: Option<String>,
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
            delete_where_clause: None,
            update_where_clause: None,
            joins: Vec::new(),
            computed_fields: Vec::new(),
            insert_owner: None,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
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

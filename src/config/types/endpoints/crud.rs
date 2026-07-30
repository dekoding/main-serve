/// CRUD-specific endpoint configuration types.
use serde::Deserialize;

use crate::db::query::types::JoinConfig;

/// Maximum possible value for CRUD page size (default and max).
pub const MAX_PAGE_SIZE: u64 = 100;

/// Minimum possible value for CRUD default page size.
pub const MIN_DEFAULT_PAGE_SIZE: u64 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// CRUD-specific configuration for an endpoint.
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
    pub pagination: super::listing::PaginationConfig,
    /// Filtering settings.
    pub filtering: super::listing::FilteringConfig,
    /// Sorting settings.
    pub sorting: super::listing::SortingConfig,
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
    /// Returns a CRUD configuration with default pagination, filtering, and sorting settings.
    fn default() -> Self {
        Self {
            table: String::new(),
            database: String::new(),
            fields: vec!["*".to_string()],
            writable_fields: Vec::new(),
            pagination: super::listing::PaginationConfig::default(),
            filtering: super::listing::FilteringConfig::default(),
            sorting: super::listing::SortingConfig::default(),
            where_clause: None,
            delete_where_clause: None,
            update_where_clause: None,
            joins: Vec::new(),
            computed_fields: Vec::new(),
            insert_owner: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
/// A computed (virtual) field defined by a SQL expression.
pub struct ComputedFieldConfig {
    /// Alias name for the computed field.
    pub name: String,
    /// SQL expression (e.g. `"COALESCE(first_name, '')"`).
    pub expression: String,
}

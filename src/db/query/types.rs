//! Types for query parameters, filters, and build results.
use serde::Deserialize;

use crate::config::types::{ComputedFieldConfig, CrudConfig, listing::SortOrder};

/// SQL join type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JoinType {
    /// INNER JOIN.
    Inner,
    /// LEFT JOIN.
    Left,
    /// RIGHT JOIN.
    Right,
}

/// Returns `JoinType::Inner` as the default SQL join type.
const fn default_join_type() -> JoinType {
    JoinType::Inner
}

/// Parameters extracted from an HTTP request for a CRUD operation.
#[derive(Debug, Default)]
pub struct QueryParams {
    /// Page number (1-indexed).
    pub page: Option<u64>,
    /// Page size.
    pub page_size: Option<u64>,
    /// Sort field name.
    pub sort: Option<String>,
    /// Sort order.
    pub order: Option<SortOrder>,
    /// Filter values: `column_name` -> value.
    pub filters: std::collections::HashMap<String, String>,
}

/// A built query ready for execution.
#[derive(Debug)]
pub struct BuiltQuery {
    /// The SQL query string.
    pub sql: String,
    /// Query parameters for parameterized execution.
    pub params: Vec<serde_json::Value>,
}

/// Context needed by insert/update/delete builders.
#[derive(Debug, Default)]
pub struct MutationContext {
    /// Fields the client is allowed to write.
    pub writable_fields: Vec<String>,
    /// Column name to set on insert for ownership tracking.
    pub insert_owner: Option<String>,
    /// WHERE clause for UPDATE queries.
    pub update_where_clause: Option<String>,
    /// WHERE clause for DELETE queries.
    pub delete_where_clause: Option<String>,
}

/// Context needed by select builders.
#[derive(Debug, Default)]
pub struct SelectContext {
    /// Fields to select (empty = all fields).
    pub fields: Vec<String>,
    /// JOIN clauses to include in the query.
    pub joins: Vec<JoinConfig>,
    /// Computed fields to include in the result.
    pub computed_fields: Vec<ComputedFieldConfig>,
    /// WHERE clause for the query.
    pub where_clause: Option<String>,
    /// Whether filtering by query parameters is enabled.
    pub filtering_enabled: bool,
    /// Fields the client may filter on (empty = none).
    pub filtering_allowed_fields: Vec<String>,
    /// Whether sorting by query parameters is enabled.
    pub sorting_enabled: bool,
    /// Default sort field (empty = primary key).
    pub sorting_default_field: String,
    /// Default sort direction.
    pub sorting_default_order: SortOrder,
    /// Fields the client may sort by (empty = none).
    pub sorting_allowed_fields: Vec<String>,
    /// Whether pagination is enabled.
    pub pagination_enabled: bool,
    /// Default number of records per page.
    pub pagination_default_page_size: u64,
    /// Maximum number of records per page.
    pub pagination_max_page_size: u64,
}

impl SelectContext {
    /// Create a `SelectContext` with permissive defaults.
    ///
    /// This is useful when the caller doesn't have a `CrudConfig` to convert from,
    /// but still needs filtering, sorting, and pagination to work with sensible defaults.
    #[must_use]
    pub fn permissive() -> Self {
        Self {
            fields: vec!["*".to_string()],
            filtering_enabled: true,
            filtering_allowed_fields: vec!["*".to_string()],
            sorting_enabled: true,
            sorting_default_field: String::new(),
            sorting_default_order: SortOrder::Desc,
            sorting_allowed_fields: vec!["*".to_string()],
            pagination_enabled: true,
            pagination_default_page_size: 20,
            pagination_max_page_size: 100,
            ..Self::default()
        }
    }
}

impl From<&CrudConfig> for MutationContext {
    /// Constructs a mutation context from the CRUD configuration's writable fields and where clauses.
    fn from(crud: &CrudConfig) -> Self {
        Self {
            writable_fields: crud.writable_fields.clone(),
            insert_owner: crud.insert_owner.clone(),
            update_where_clause: crud.update_where_clause.clone(),
            delete_where_clause: crud.delete_where_clause.clone(),
        }
    }
}

impl From<&CrudConfig> for SelectContext {
    /// Constructs a select context from the CRUD configuration's field and filter settings.
    fn from(crud: &CrudConfig) -> Self {
        Self {
            fields: crud.fields.clone(),
            joins: crud.joins.clone(),
            computed_fields: crud.computed_fields.clone(),
            where_clause: crud.where_clause.clone(),
            filtering_enabled: crud.filtering.enabled,
            filtering_allowed_fields: crud.filtering.allowed_fields.clone(),
            sorting_enabled: crud.sorting.enabled,
            sorting_default_field: crud.sorting.default_field.clone(),
            sorting_default_order: crud.sorting.default_order,
            sorting_allowed_fields: crud.sorting.allowed_fields.clone(),
            pagination_enabled: crud.pagination.enabled,
            pagination_default_page_size: crud.pagination.default_page_size,
            pagination_max_page_size: crud.pagination.max_page_size,
        }
    }
}

/// Join configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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

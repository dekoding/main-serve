use serde::Deserialize;

use crate::config::types::{ComputedFieldConfig, SortOrder};

/// SQL join type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JoinType {
    Inner,
    Left,
    Right,
}

fn default_join_type() -> JoinType {
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
    pub order: Option<crate::config::types::SortOrder>,
    /// Filter values: `column_name` -> value.
    pub filters: std::collections::HashMap<String, String>,
}

/// A built query ready for execution.
#[derive(Debug)]
pub struct BuiltQuery {
    pub sql: String,
    pub params: Vec<serde_json::Value>,
}

/// Context needed by insert/update/delete builders.
#[derive(Debug, Default)]
pub struct MutationContext {
    pub writable_fields: Vec<String>,
    pub insert_owner: Option<String>,
    pub update_where_clause: Option<String>,
    pub delete_where_clause: Option<String>,
}

/// Context needed by select builders.
#[derive(Debug, Default)]
pub struct SelectContext {
    pub fields: Vec<String>,
    pub joins: Vec<JoinConfig>,
    pub computed_fields: Vec<ComputedFieldConfig>,
    pub where_clause: Option<String>,
    pub filtering_enabled: bool,
    pub filtering_allowed_fields: Vec<String>,
    pub sorting_enabled: bool,
    pub sorting_default_field: String,
    pub sorting_default_order: SortOrder,
    pub sorting_allowed_fields: Vec<String>,
    pub pagination_enabled: bool,
    pub pagination_default_page_size: u64,
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

/// Join configuration.
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

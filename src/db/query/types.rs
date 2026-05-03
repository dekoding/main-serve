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

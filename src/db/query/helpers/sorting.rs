use crate::{
    config::types::listing::SortOrder,
    db::query::{
        helpers::{
            column_exists, is_bracket_notation, is_jsonb_column, is_dotted_path, parse_sort_field,
        },
        types::QueryParams,
    },
};

/// Validate that a sort field exists in the table schema.
/// Returns true if the field is valid (either a regular column or a JSONB nested path).
#[must_use]
pub(crate) fn is_valid_sort_field(
    field: &str,
    columns: &[crate::config::types::ColumnConfig],
) -> bool {
    let (base, _) = parse_sort_field(field);
    // Allow bracket notation (e.g., metadata[role])
    if is_bracket_notation(field) {
        is_jsonb_column(&base, columns) || column_exists(&base, columns)
    } else if is_dotted_path(field) {
        // For JSONB paths, check if base column is a JSON/JSONB column
        is_jsonb_column(&base, columns)
    } else {
        // Regular field
        column_exists(field, columns)
    }
}

/// Extract pagination, sorting, and filter params from a query string map.
///
/// Reserved keys (`page`, `page_size`, `per_page`, `sort`, `order`) are treated
/// as query parameters; all remaining keys are collected as filters.
#[must_use]
pub(crate) fn extract_query_params(qs: &std::collections::HashMap<String, String>) -> QueryParams {
    let page = qs.get("page").and_then(|v| v.parse::<u64>().ok());
    let page_size = qs
        .get("page_size")
        .or_else(|| qs.get("per_page"))
        .and_then(|v| v.parse::<u64>().ok());
    let sort = qs.get("sort").cloned();
    let order = qs.get("order").and_then(|v| match v.as_str() {
        "asc" | "ASC" => Some(SortOrder::Asc),
        "desc" | "DESC" => Some(SortOrder::Desc),
        _ => None,
    });

    let reserved = ["page", "page_size", "per_page", "sort", "order"];
    let filters: std::collections::HashMap<String, String> = qs
        .iter()
        .filter(|(k, _)| !reserved.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    QueryParams {
        page,
        page_size,
        sort,
        order,
        filters,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{ColumnConfig, ColumnType};

    fn sample_table() -> std::vec::Vec<ColumnConfig> {
        vec![
            ColumnConfig {
                name: "id".to_string(),
                column_type: ColumnType::Integer,
                primary_key: true,
                nullable: false,
                ..Default::default()
            },
            ColumnConfig {
                name: "title".to_string(),
                column_type: ColumnType::Text,
                nullable: false,
                ..Default::default()
            },
            ColumnConfig {
                name: "author".to_string(),
                column_type: ColumnType::Varchar,
                nullable: false,
                ..Default::default()
            },
        ]
    }

    fn sample_jsonb_table() -> Vec<ColumnConfig> {
        vec![
            ColumnConfig {
                name: "id".to_string(),
                column_type: ColumnType::Integer,
                primary_key: true,
                nullable: false,
                ..Default::default()
            },
            ColumnConfig {
                name: "title".to_string(),
                column_type: ColumnType::Text,
                nullable: false,
                ..Default::default()
            },
            ColumnConfig {
                name: "metadata".to_string(),
                column_type: ColumnType::Jsonb,
                nullable: true,
                ..Default::default()
            },
        ]
    }

    #[test]
    fn test_is_valid_sort_field_regular_column() {
        let cols = sample_table();
        assert!(is_valid_sort_field("title", &cols));
        assert!(is_valid_sort_field("author", &cols));
    }

    #[test]
    fn test_is_valid_sort_field_jsonb_path() {
        let cols = sample_jsonb_table();
        assert!(is_valid_sort_field("metadata.role", &cols));
    }

    #[test]
    fn test_is_valid_sort_field_invalid_column() {
        let cols = sample_table();
        assert!(!is_valid_sort_field("nonexistent", &cols));
    }

    #[test]
    fn test_is_valid_sort_field_jsonb_path_invalid_base() {
        let cols = sample_jsonb_table();
        assert!(!is_valid_sort_field("nonexistent.role", &cols));
    }

    // -- extract_query_params --

    #[test]
    fn test_extract_query_params_reserved_keys() {
        let mut qs = std::collections::HashMap::new();
        qs.insert("page".to_string(), "2".to_string());
        qs.insert("page_size".to_string(), "10".to_string());
        qs.insert("sort".to_string(), "created_at".to_string());
        qs.insert("order".to_string(), "desc".to_string());
        qs.insert("title".to_string(), "hello".to_string());

        let params = extract_query_params(&qs);
        assert_eq!(params.page, Some(2));
        assert_eq!(params.page_size, Some(10));
        assert_eq!(params.sort, Some("created_at".to_string()));
        assert_eq!(params.order, Some(SortOrder::Desc));
        assert_eq!(params.filters.len(), 1);
        assert_eq!(params.filters.get("title"), Some(&"hello".to_string()));
    }

    #[test]
    fn test_extract_query_params_per_page_alias() {
        let mut qs = std::collections::HashMap::new();
        qs.insert("per_page".to_string(), "25".to_string());
        qs.insert("page_size".to_string(), "10".to_string());

        let params = extract_query_params(&qs);
        // page_size takes precedence over per_page
        assert_eq!(params.page_size, Some(10));
    }

    #[test]
    fn test_extract_query_params_case_insensitive_order() {
        let mut qs = std::collections::HashMap::new();
        qs.insert("order".to_string(), "ASC".to_string());

        let params = extract_query_params(&qs);
        assert_eq!(params.order, Some(SortOrder::Asc));
    }

    #[test]
    fn test_extract_query_params_no_reserved_keys() {
        let mut qs = std::collections::HashMap::new();
        qs.insert("title".to_string(), "hello".to_string());
        qs.insert("author".to_string(), "world".to_string());

        let params = extract_query_params(&qs);
        assert!(params.page.is_none());
        assert!(params.page_size.is_none());
        assert!(params.sort.is_none());
        assert!(params.order.is_none());
        assert_eq!(params.filters.len(), 2);
    }
}

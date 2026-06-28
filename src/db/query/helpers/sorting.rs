use crate::db::query::types::QueryParams;

/// Parse a sorting field that may use LHS bracket notation.
///
/// Handles two syntaxes:
/// - Dot notation: `metadata.role` -> base="metadata", path=["role"]
/// - LHS brackets: `metadata[role]` -> base="metadata", path=["role"]
/// - Nested: `metadata.user[profile].email` -> base="metadata", path=["user","profile","email"]
///
/// Returns (`base_column`, `path_segments`) where `path_segments` are the nested field names.
#[must_use]
pub(crate) fn parse_sort_field(field: &str) -> (String, Vec<String>) {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    let chars: Vec<char> = field.chars().collect();

    while i < chars.len() {
        let c = chars[i];

        if c == '[' {
            // End of current segment, start of bracket notation
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
            i += 1;
            let mut bracket_content = String::new();
            while i < chars.len() && chars[i] != ']' {
                bracket_content.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                // Skip the closing bracket
                i += 1;
            }
            if !bracket_content.is_empty() {
                parts.push(bracket_content);
            }
        } else if c == '.' {
            // End of current segment
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
            i += 1;
        } else {
            current.push(c);
            i += 1;
        }
    }

    // Add any remaining content
    if !current.is_empty() {
        parts.push(current);
    }

    // First segment is the base column name
    if parts.is_empty() {
        return (field.to_string(), Vec::new());
    }

    let base = parts[0].clone();
    let path = parts[1..].to_vec();
    (base, path)
}

/// Check if a sort field uses bracket notation (e.g., `field[key]`).
#[must_use]
pub(crate) fn is_bracket_notation(field: &str) -> bool {
    field.contains('[')
}

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
    } else if super::filters::is_jsonb_path(field) {
        // For JSONB paths, check if base column is a JSON/JSONB column
        is_jsonb_column(&base, columns)
    } else {
        // Regular field
        column_exists(field, columns)
    }
}

/// Check if a column in the table config is a JSON or JSONB type.
fn is_jsonb_column(column_name: &str, columns: &[crate::config::types::ColumnConfig]) -> bool {
    columns.iter().any(|c| {
        c.name == column_name
            && matches!(
                c.column_type,
                crate::config::types::ColumnType::Json | crate::config::types::ColumnType::Jsonb
            )
    })
}

/// Check if a filter column exists in the table schema.
fn column_exists(column_name: &str, columns: &[crate::config::types::ColumnConfig]) -> bool {
    columns.iter().any(|c| c.name == column_name)
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
        "asc" | "ASC" => Some(crate::config::types::SortOrder::Asc),
        "desc" | "DESC" => Some(crate::config::types::SortOrder::Desc),
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

    // -- parse_sort_field --

    #[test]
    fn test_parse_sort_field_simple() {
        let (base, path) = parse_sort_field("title");
        assert_eq!(base, "title");
        assert!(path.is_empty());
    }

    #[test]
    fn test_parse_sort_field_dot_notation() {
        let (base, path) = parse_sort_field("metadata.role");
        assert_eq!(base, "metadata");
        assert_eq!(path, vec!["role"]);
    }

    #[test]
    fn test_parse_sort_field_bracket_notation() {
        let (base, path) = parse_sort_field("metadata[role]");
        assert_eq!(base, "metadata");
        assert_eq!(path, vec!["role"]);
    }

    #[test]
    fn test_parse_sort_field_nested_mixed() {
        let (base, path) = parse_sort_field("metadata.user[profile].email");
        assert_eq!(base, "metadata");
        assert_eq!(path, vec!["user", "profile", "email"]);
    }

    #[test]
    fn test_parse_sort_field_deep_nested() {
        let (base, path) = parse_sort_field("a.b.c.d");
        assert_eq!(base, "a");
        assert_eq!(path, vec!["b", "c", "d"]);
    }

    // -- is_bracket_notation --

    #[test]
    fn test_is_bracket_notation_true() {
        assert!(is_bracket_notation("metadata[role]"));
        assert!(is_bracket_notation("a[b][c]"));
    }

    #[test]
    fn test_is_bracket_notation_false() {
        assert!(!is_bracket_notation("metadata.role"));
        assert!(!is_bracket_notation("title"));
    }

    // -- is_valid_sort_field --

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
        assert_eq!(params.order, Some(crate::config::types::SortOrder::Desc));
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
        assert_eq!(params.order, Some(crate::config::types::SortOrder::Asc));
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

//! Filter expression parsing, validation, and SQL generation.
use crate::config::types::ColumnConfig;
use crate::db::query::helpers::compile_regex;
use crate::db::query::helpers::{
    column_exists, is_bracket_notation, is_dotted_path, is_jsonb_column, parse_sort_field,
};
use crate::error::AppError;
use std::sync::LazyLock;

/// Comparison operators supported in filter expressions.
///
/// Maps query parameter operators (eq, ne, gt, gte, lt, lte, in, `not_in`,
/// contains, exists, startswith, endswith, like, ilike) to typed variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOperator {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    In,
    NotIn,
    Contains,
    Exists,
    StartsWith,
    EndsWith,
    Like,
    ILike,
}

/// A parsed filter expression with a field path and comparison operator.
#[derive(Debug)]
pub struct FilterExpression {
    pub path: Vec<String>,
    pub operator: FilterOperator,
}

/// Compiled regex for parsing bracket-notation filter keys like `field[operator]`.
///
/// The pattern is a compile-time constant that has been tested in CI.
static FILTER_KEY_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| compile_regex(r"^(.*)\[([a-z_]+)\]$"));

/// Parse a filter key like `metadata.user.age[gt]` into a `FilterExpression`.
pub fn parse_filter_key(key: &str) -> Result<FilterExpression, AppError> {
    let re = &*FILTER_KEY_RE;
    let (path_str, operator_str) = if let Some(caps) = re.captures(key) {
        let path_str = caps.get(1).map_or(key, |m| m.as_str());
        let operator_str = caps.get(2).map_or("eq", |m| m.as_str());
        (path_str, operator_str)
    } else {
        // Check if the key contains an unclosed bracket or invalid bracket notation
        if key.contains('[') {
            return Err(AppError::BadRequest(format!(
                "Invalid filter key syntax: '{key}'. Bracket notation must be properly closed. Use format like 'field[operator]' where operator is one of: eq, ne, gt, gte, lt, lte, in, not_in, contains, exists, startswith, endswith, like, ilike"
            )));
        }
        if key.contains(']') {
            return Err(AppError::BadRequest(format!(
                "Invalid filter key syntax: '{key}'. Unexpected closing bracket without opening bracket."
            )));
        }
        (key, "eq")
    };

    let operator = match operator_str {
        "eq" => FilterOperator::Eq,
        "ne" => FilterOperator::Ne,
        "gt" => FilterOperator::Gt,
        "gte" => FilterOperator::Gte,
        "lt" => FilterOperator::Lt,
        "lte" => FilterOperator::Lte,
        "in" => FilterOperator::In,
        "not_in" => FilterOperator::NotIn,
        "contains" => FilterOperator::Contains,
        "exists" => FilterOperator::Exists,
        "startswith" => FilterOperator::StartsWith,
        "endswith" => FilterOperator::EndsWith,
        "like" => FilterOperator::Like,
        "ilike" => FilterOperator::ILike,
        _ => {
            return Err(AppError::BadRequest(format!(
                "Unsupported operator: {operator_str}"
            )));
        }
    };

    let path: Vec<String> = path_str
        .split('.')
        .map(std::string::ToString::to_string)
        .collect();

    Ok(FilterExpression { path, operator })
}

/// Extract the base column name from a field string that may contain `JSONPath` or operators.
/// Handles cases like:
/// - `$.metadata.role` -> `metadata`
/// - `metadata->>'role'` -> `metadata`
/// - `metadata#>'{user,role}'` -> `metadata`
/// - `metadata.role` -> `metadata`
#[must_use]
pub fn extract_base_column(field: &str) -> String {
    // Handle JSONPath syntax starting with $
    if field.starts_with('$') {
        // Extract column name from $.metadata.role or $.metadata.tags[0]
        let parts: Vec<&str> = field.split('.').collect();
        if parts.len() > 1 {
            // $.metadata -> metadata
            parts[1..2]
                .first()
                .map_or(field, |s| s.trim_start_matches('$'))
                .to_string()
        } else {
            field.trim_start_matches('$').to_string()
        }
    } else if field.contains("->") || field.contains("#>") || field.contains("#>>") {
        // Extract column name from operator syntax like metadata->>'role'
        // Split on -> and #> but keep the first part before any of these operators
        let parts: Vec<&str> = field.split(|c| ['>', '#'].contains(&c)).collect();
        parts.first().map_or_else(
            || field.to_string(),
            |first| first.trim_end_matches('-').to_string(),
        )
    } else if is_bracket_notation(field) {
        // Extract base from bracket notation
        let (base, _) = parse_sort_field(field);
        base
    } else if is_dotted_path(field) {
        // Extract base from dot notation
        let (base, _) = parse_sort_field(field);
        base
    } else {
        // Regular field name
        field.to_string()
    }
}

/// Extract the JSONB path string from a dotted field name or bracket notation.
/// Converts `metadata.role` to `$.role` and `metadata[role]` to `$.role`.
/// Converts `metadata.user.profile` to `$.user.profile` and `metadata[user][profile]` to `$.user.profile`.
#[must_use]
pub fn extract_jsonb_path(field: &str) -> String {
    let (_, path) = parse_sort_field(field);
    if path.is_empty() {
        return "$".to_string();
    }
    format!("$.{}", path.join("."))
}

/// Validate that a filter column exists in the table schema.
/// Returns true if the column is valid (either a regular column or a JSONB column).
#[must_use]
pub fn is_valid_filter_column(field: &str, columns: &[ColumnConfig]) -> bool {
    let base = extract_base_column(field);
    is_jsonb_column(&base, columns) || column_exists(&base, columns)
}

#[cfg(test)]
mod tests {
    use super::super::interpolation::build_filter_param;
    use super::*;
    use crate::config::types::{ColumnType, DatabaseDriver, TableConfig};

    fn sample_table() -> TableConfig {
        TableConfig {
            name: "posts".to_string(),
            database: "main".to_string(),
            columns: vec![
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
            ],
            foreign_keys: vec![],
        }
    }

    fn sample_jsonb_table() -> TableConfig {
        TableConfig {
            name: "articles".to_string(),
            database: "main".to_string(),
            columns: vec![
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
            ],
            foreign_keys: vec![],
        }
    }

    // -- parse_filter_key --

    #[test]
    fn test_parse_filter_key_simple_eq() {
        let expr = parse_filter_key("title").unwrap();
        assert_eq!(expr.path, vec!["title"]);
        assert_eq!(expr.operator, FilterOperator::Eq);
    }

    #[test]
    fn test_parse_filter_key_with_operator() {
        let expr = parse_filter_key("age[gt]").unwrap();
        assert_eq!(expr.path, vec!["age"]);
        assert_eq!(expr.operator, FilterOperator::Gt);
    }

    #[test]
    fn test_parse_filter_key_jsonb_nested() {
        let expr = parse_filter_key("metadata.role[eq]").unwrap();
        assert_eq!(expr.path, vec!["metadata", "role"]);
        assert_eq!(expr.operator, FilterOperator::Eq);
    }

    #[test]
    fn test_parse_filter_key_all_operators() {
        for (key, expected_op) in [
            ("f[ne]", FilterOperator::Ne),
            ("f[gte]", FilterOperator::Gte),
            ("f[lt]", FilterOperator::Lt),
            ("f[lte]", FilterOperator::Lte),
            ("f[in]", FilterOperator::In),
            ("f[not_in]", FilterOperator::NotIn),
            ("f[contains]", FilterOperator::Contains),
            ("f[exists]", FilterOperator::Exists),
            ("f[startswith]", FilterOperator::StartsWith),
            ("f[endswith]", FilterOperator::EndsWith),
            ("f[like]", FilterOperator::Like),
            ("f[ilike]", FilterOperator::ILike),
        ] {
            let expr = parse_filter_key(key).unwrap();
            assert_eq!(expr.operator, expected_op, "operator for key '{key}'");
        }
    }

    #[test]
    fn test_parse_filter_key_unclosed_bracket() {
        let err = parse_filter_key("field[gt").unwrap_err();
        assert!(err.to_string().contains("Invalid filter key"));
    }

    #[test]
    fn test_parse_filter_key_closing_bracket_only() {
        let err = parse_filter_key("field]").unwrap_err();
        assert!(err.to_string().contains("Invalid filter key"));
    }

    #[test]
    fn test_parse_filter_key_invalid_operator() {
        let err = parse_filter_key("field[foo]").unwrap_err();
        assert!(err.to_string().contains("Unsupported operator"));
    }

    // -- extract_base_column --

    #[test]
    fn test_extract_base_column_simple() {
        assert_eq!(extract_base_column("title"), "title");
    }

    #[test]
    fn test_extract_base_column_dot_notation() {
        assert_eq!(extract_base_column("metadata.role"), "metadata");
    }

    #[test]
    fn test_extract_base_column_bracket_notation() {
        assert_eq!(extract_base_column("metadata[role]"), "metadata");
    }

    #[test]
    fn test_extract_base_column_jsonpath() {
        assert_eq!(extract_base_column("$.metadata.role"), "metadata");
    }

    #[test]
    fn test_extract_base_column_arrow_notation() {
        assert_eq!(extract_base_column("metadata->>'role'"), "metadata");
    }

    #[test]
    fn test_extract_base_column_hash_notation() {
        assert_eq!(extract_base_column("metadata#>>'{role}'"), "metadata");
    }

    // -- extract_jsonb_path --

    #[test]
    fn test_extract_jsonb_path_single() {
        assert_eq!(extract_jsonb_path("metadata.role"), "$.role");
    }

    #[test]
    fn test_extract_jsonb_path_nested() {
        assert_eq!(
            extract_jsonb_path("metadata.user.profile"),
            "$.user.profile"
        );
    }

    #[test]
    fn test_extract_jsonb_path_bracket() {
        assert_eq!(extract_jsonb_path("metadata[role]"), "$.role");
    }

    #[test]
    fn test_extract_jsonb_path_no_path() {
        assert_eq!(extract_jsonb_path("metadata"), "$");
    }

    // -- is_jsonb_column --

    #[test]
    fn test_is_jsonb_column_true() {
        let table = sample_jsonb_table();
        assert!(is_jsonb_column("metadata", &table.columns));
    }

    #[test]
    fn test_is_jsonb_column_false() {
        let table = sample_jsonb_table();
        assert!(!is_jsonb_column("title", &table.columns));
        assert!(!is_jsonb_column("nonexistent", &table.columns));
    }

    // -- column_exists --

    #[test]
    fn test_column_exists_found() {
        let table = sample_table();
        assert!(column_exists("title", &table.columns));
        assert!(column_exists("id", &table.columns));
    }

    #[test]
    fn test_column_exists_not_found() {
        let table = sample_table();
        assert!(!column_exists("nonexistent", &table.columns));
    }

    // -- is_valid_filter_column --

    #[test]
    fn test_is_valid_filter_column_regular() {
        let table = sample_table();
        assert!(is_valid_filter_column("title", &table.columns));
    }

    #[test]
    fn test_is_valid_filter_column_jsonb() {
        let table = sample_jsonb_table();
        assert!(is_valid_filter_column("metadata", &table.columns));
    }

    #[test]
    fn test_is_valid_filter_column_invalid() {
        let table = sample_table();
        assert!(!is_valid_filter_column("nonexistent", &table.columns));
    }

    // -- build_filter_param --

    #[test]
    fn test_build_filter_param_with_type() {
        let table = sample_jsonb_table();
        let meta_col = table.columns.iter().find(|c| c.name == "metadata").unwrap();
        let val = build_filter_param("true", meta_col.column_type, DatabaseDriver::Sqlite);
        assert_eq!(val, serde_json::json!(true));
    }

    // -- Integration: full filter key to SQL pipeline --

    #[test]
    fn test_filter_pipeline_simple_eq() {
        let expr = parse_filter_key("title[eq]").unwrap();
        assert_eq!(expr.path, vec!["title"]);
        assert_eq!(expr.operator, FilterOperator::Eq);
    }

    #[test]
    fn test_filter_pipeline_jsonb_nested() {
        let expr = parse_filter_key("metadata.role[eq]").unwrap();
        assert_eq!(expr.path, vec!["metadata", "role"]);
        assert_eq!(expr.operator, FilterOperator::Eq);
        let jsonb_path = extract_jsonb_path("metadata.role");
        assert_eq!(jsonb_path, "$.role");
    }

    #[test]
    fn test_filter_pipeline_bracket_notation() {
        // Bracket notation in the key name is preserved as a literal
        // segment; only the last [operator] is stripped.
        let expr = parse_filter_key("metadata[role][eq]").unwrap();
        assert_eq!(expr.path, vec!["metadata[role]"]);
        assert_eq!(expr.operator, FilterOperator::Eq);
    }
}

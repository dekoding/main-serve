//! Shared query helper utilities for column existence checks and identifier quoting.
use crate::config::types::{ColumnConfig, DatabaseDriver};

/// Check if a column in the table config is a JSON or JSONB type.
#[must_use]
pub fn is_jsonb_column(column_name: &str, columns: &[ColumnConfig]) -> bool {
    columns.iter().any(|c| {
        c.name == column_name
            && matches!(
                c.column_type,
                crate::config::types::ColumnType::Json | crate::config::types::ColumnType::Jsonb
            )
    })
}

/// Check if a filter column exists in the table schema.
#[must_use]
pub fn column_exists(column_name: &str, columns: &[ColumnConfig]) -> bool {
    columns.iter().any(|c| c.name == column_name)
}

/// Generate a driver-appropriate parameter placeholder.
#[must_use]
pub fn placeholder(driver: DatabaseDriver, index: usize) -> String {
    match driver {
        DatabaseDriver::Postgres => format!("${index}"),
        DatabaseDriver::Sqlite | DatabaseDriver::Mysql => "?".to_string(),
    }
}

/// Generate a driver-appropriate `NOW()` equivalent expression.
///
/// `SQLite` uses `CURRENT_TIMESTAMP`, `PostgreSQL` and `MySQL` use `NOW()`.
#[must_use]
pub const fn now_expr(driver: DatabaseDriver) -> &'static str {
    match driver {
        DatabaseDriver::Sqlite => "CURRENT_TIMESTAMP",
        _ => "NOW()",
    }
}

/// Parse a sorting field that may use LHS bracket notation.
///
/// Handles two syntaxes:
/// - Dot notation: `metadata.role` -> base="metadata", path=`["role"]`
/// - LHS brackets: `metadata[role]` -> base="metadata", path=`["role"]`
/// - Nested: `metadata.user[profile].email` -> base="metadata", path=`["user","profile","email"]`
///
/// Returns (`base_column`, `path_segments`) where `path_segments` are the nested field names.
#[must_use]
pub fn parse_sort_field(field: &str) -> (String, Vec<String>) {
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
                // SAFETY: i < chars.len() is checked in the while loop condition.
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
pub fn is_bracket_notation(field: &str) -> bool {
    field.contains('[')
}

/// Check if a field path is a nested path (contains dots).
#[must_use]
pub fn is_dotted_path(field: &str) -> bool {
    field.contains('.')
}

/// Quote an object name (table or index) for the current driver.
#[inline]
pub fn quote_identifier(name: &str, driver: DatabaseDriver) -> String {
    match driver {
        DatabaseDriver::Mysql => format!("`{name}`"),
        DatabaseDriver::Sqlite | DatabaseDriver::Postgres => format!("\"{name}\""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // -- is_dotted_path --

    #[test]
    fn test_is_dotted_path_true() {
        assert!(is_dotted_path("metadata.role"));
        assert!(is_dotted_path("a.b.c"));
    }

    #[test]
    fn test_is_dotted_path_false() {
        assert!(!is_dotted_path("title"));
        assert!(!is_dotted_path("id"));
    }

    // -- quote_identifier --

    #[test]
    fn test_quote_identifier_sqlite() {
        assert_eq!(
            quote_identifier("users", DatabaseDriver::Sqlite),
            r#""users""#
        );
        assert_eq!(
            quote_identifier("table_name", DatabaseDriver::Sqlite),
            r#""table_name""#
        );
    }

    #[test]
    fn test_quote_identifier_postgres() {
        assert_eq!(
            quote_identifier("users", DatabaseDriver::Postgres),
            r#""users""#
        );
        assert_eq!(
            quote_identifier("table.name", DatabaseDriver::Postgres),
            r#""table.name""#
        );
    }

    #[test]
    fn test_quote_identifier_mysql() {
        assert_eq!(quote_identifier("users", DatabaseDriver::Mysql), "`users`");
        assert_eq!(
            quote_identifier("table_name", DatabaseDriver::Mysql),
            "`table_name`"
        );
    }
}

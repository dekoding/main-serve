use crate::config::types::{ColumnConfig, DatabaseDriver};
use crate::error::AppError;
use crate::middleware::auth::extractor::RequestContext;

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

#[derive(Debug)]
pub struct FilterExpression {
    pub path: Vec<String>,
    pub operator: FilterOperator,
}

/// Parse a filter key like `metadata.user.age[gt]` into a `FilterExpression`.
pub fn parse_filter_key(key: &str) -> Result<FilterExpression, AppError> {
    use regex::Regex;
    let re = Regex::new(r"^(.*)\[([a-z_]+)\]$")
        .map_err(|e| AppError::Internal(format!("Invalid regex: {e}")))?;

    let (path_str, operator_str) = if let Some(caps) = re.captures(key) {
        (caps.get(1).unwrap().as_str(), caps.get(2).unwrap().as_str())
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

    let path: Vec<String> = path_str.split('.').map(|s| s.to_string()).collect();

    Ok(FilterExpression { path, operator })
}

/// Helper to interpolate a string value.
pub fn interpolate_value(
    value: &str,
    context: &RequestContext,
) -> Result<serde_json::Value, AppError> {
    use regex::Regex;
    let re = Regex::new(r"\$\{([^}]+)\}")
        .map_err(|e| AppError::Internal(format!("Invalid interpolation regex: {e}")))?;

    // If there are no matches, return the original string as a JSON value.
    if !re.is_match(value) {
        return Ok(serde_json::Value::String(value.to_string()));
    }

    // If there are matches, we need to perform the interpolation.
    let mut last_match_end = 0;
    let mut new_string = String::new();
    let mut found_resolution = false;

    for cap in re.captures_iter(value) {
        let full_match = cap
            .get(0)
            .ok_or_else(|| AppError::Internal("Regex match failed".to_string()))?;
        let key = cap
            .get(1)
            .ok_or_else(|| AppError::Internal("Regex capture failed".to_string()))?
            .as_str();

        new_string.push_str(&value[last_match_end..full_match.start()]);

        // Check for default values: ${key:-default}
        let resolved_value = if key.contains(":-") {
            if let Some(idx) = key.find(":-") {
                let base_key = &key[..idx];
                let default_val = &key[idx + 2..];
                resolve_single_key(base_key, context).or(Some(default_val.to_string()))
            } else {
                None
            }
        } else {
            resolve_single_key(key, context)
        };

        if let Some(val) = resolved_value {
            new_string.push_str(&val);
            found_resolution = true;
        } else {
            // If it couldn't be resolved, keep the original placeholder.
            new_string.push_str(full_match.as_str());
        }

        last_match_end = full_match.end();
    }

    new_string.push_str(&value[last_match_end..]);

    if found_resolution {
        Ok(serde_json::Value::String(new_string))
    } else {
        Ok(serde_json::Value::String(value.to_string()))
    }
}

/// Helper to resolve a single context key.
pub fn resolve_single_key(key: &str, context: &RequestContext) -> Option<String> {
    if key == "request.user.id" {
        context.user_id.clone()
    } else if key == "request.user.role" {
        context.user_role.clone()
    } else if key == "request.method" {
        Some(context.method.clone())
    } else if key == "request.path" {
        Some(context.path.clone())
    } else if let Some(header_name) = key.strip_prefix("request.headers.") {
        context.headers.get(header_name).cloned()
    } else if let Some(query_key) = key.strip_prefix("request.query.") {
        context.query_params.get(query_key).cloned()
    } else {
        None
    }
}

/// Generate a driver-appropriate parameter placeholder.
pub fn placeholder(driver: DatabaseDriver, index: usize) -> String {
    match driver {
        DatabaseDriver::Postgres => format!("${index}"),
        DatabaseDriver::Sqlite | DatabaseDriver::Mysql => "?".to_string(),
    }
}

/// Resolve writable fields: ["*"] -> all column names, otherwise as-is.
pub fn resolve_writable_fields(
    writable: &[String],
    table: &crate::config::types::TableConfig,
) -> std::collections::HashSet<String> {
    if writable.contains(&"*".to_string()) {
        table.columns.iter().map(|c| c.name.clone()).collect()
    } else {
        writable.iter().cloned().collect()
    }
}

/// Resolve field list: ["*"] -> all column names, otherwise as-is.
pub fn resolve_fields(fields: &[String], table: &crate::config::types::TableConfig) -> Vec<String> {
    if fields.len() == 1 && fields[0] == "*" {
        table.columns.iter().map(|c| c.name.clone()).collect()
    } else {
        fields.to_vec()
    }
}

/// Find the primary key column name.
pub fn find_pk_column(table: &crate::config::types::TableConfig) -> Result<String, AppError> {
    table
        .columns
        .iter()
        .find(|c| c.primary_key)
        .map(|c| c.name.clone())
        .ok_or_else(|| AppError::Internal("Table has no primary key column".to_string()))
}

/// Coerce a PK value from a URL path segment (always a string) into the
/// appropriate `serde_json::Value` based on the column's declared type.
///
/// `PostgreSQL` requires bind parameters to match the column type exactly;
/// binding a string `"1"` against an integer column causes a type error.
pub fn coerce_pk_value(table: &crate::config::types::TableConfig, raw: &str) -> serde_json::Value {
    let col_type = table
        .columns
        .iter()
        .find(|c| c.primary_key)
        .map(|c| c.column_type);

    match col_type {
        Some(
            crate::config::types::ColumnType::Integer
            | crate::config::types::ColumnType::Bigint
            | crate::config::types::ColumnType::Smallint
            | crate::config::types::ColumnType::Serial
            | crate::config::types::ColumnType::Bigserial,
        ) => raw.parse::<i64>().map_or_else(
            |_| serde_json::Value::Number(serde_json::Number::from(i64::MIN)),
            |n| serde_json::json!(n),
        ),
        Some(
            crate::config::types::ColumnType::Float
            | crate::config::types::ColumnType::Double
            | crate::config::types::ColumnType::Decimal,
        ) => raw.parse::<f64>().map_or_else(
            |_| serde_json::Value::String(raw.to_string()),
            |n| serde_json::json!(n),
        ),
        _ => serde_json::Value::String(raw.to_string()),
    }
}

/// Coerce a filter value to the appropriate `serde_json::Value` based on type inference.
///
/// This function attempts to parse string filter values into their appropriate JSON types
/// (number, boolean, null, or string) to avoid type mismatch errors in databases that
/// expect specific types. For example, comparing a JSONB number column to a string value
/// will fail in PostgreSQL without proper type coercion.
///
/// The coercion follows this priority order:
/// 1. `null` literal -> `serde_json::Value::Null`
/// 2. `true`/`false` -> `serde_json::Value::Bool`
/// 3. Integer numbers (e.g., "123") -> `serde_json::Value::Number`
/// 4. Floating point numbers (e.g., "123.45") -> `serde_json::Value::Number`
/// 5. Everything else -> `serde_json::Value::String`
///
/// # Arguments
///
/// * `value` - The string value from the filter query parameter
///
/// # Returns
///
/// The value coerced to the most appropriate `serde_json::Value` type.
pub fn coerce_filter_value(value: &str) -> serde_json::Value {
    // Handle null explicitly
    if value.to_lowercase() == "null" {
        return serde_json::Value::Null;
    }

    // Handle booleans
    if let Ok(bool_val) = value.parse::<bool>() {
        return serde_json::Value::Bool(bool_val);
    }

    // Try parsing as integer first
    if let Ok(int_val) = value.parse::<i64>() {
        return serde_json::Value::Number(int_val.into());
    }

    // Try parsing as floating point
    if let Ok(float_val) = value.parse::<f64>() {
        return serde_json::Number::from_f64(float_val)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::String(value.to_string()));
    }

    // Default to string
    serde_json::Value::String(value.to_string())
}

/// Coerce a filter value based on the column's declared type.
///
/// This function is similar to `coerce_filter_value` but also takes into account
/// the column's declared type in the table schema. This is important for cases
/// where the user explicitly wants to filter by a string that happens to look
/// like a number (e.g., filtering a text column for the value "123").
///
/// For JSON/JSONB columns, the coercion strategy depends on the database driver:
/// - SQLite: `json_extract` preserves native JSON types, so try parsing as JSON.
/// - PostgreSQL: `#>>` always returns text, so always coerce to string.
/// - MySQL: `JSON_UNQUOTE(JSON_EXTRACT(...))` always returns text, so always coerce to string.
///
/// # Arguments
///
/// * `value` - The string value from the filter query parameter
/// * `column_type` - The declared type of the column being filtered
/// * `driver` - Database driver to determine coercion strategy
///
/// # Returns
///
/// The value coerced to the appropriate `serde_json::Value` type based on both
/// the value itself and the column type.
pub fn coerce_filter_value_by_type(
    value: &str,
    column_type: &crate::config::types::ColumnType,
    driver: DatabaseDriver,
) -> serde_json::Value {
    match column_type {
        crate::config::types::ColumnType::Integer
        | crate::config::types::ColumnType::Bigint
        | crate::config::types::ColumnType::Smallint
        | crate::config::types::ColumnType::Serial
        | crate::config::types::ColumnType::Bigserial => {
            // For integer columns, try to parse as integer first
            value.parse::<i64>().map_or_else(
                |_| serde_json::Value::String(value.to_string()),
                |n| serde_json::json!(n),
            )
        }
        crate::config::types::ColumnType::Float
        | crate::config::types::ColumnType::Double
        | crate::config::types::ColumnType::Decimal => {
            // For float columns, try to parse as f64 first
            value.parse::<f64>().map_or_else(
                |_| serde_json::Value::String(value.to_string()),
                |n| serde_json::json!(n),
            )
        }
        crate::config::types::ColumnType::Boolean => {
            // For boolean columns, try to parse as bool first
            value.parse::<bool>().map_or_else(
                |_| serde_json::Value::String(value.to_string()),
                serde_json::Value::Bool,
            )
        }
        crate::config::types::ColumnType::Json | crate::config::types::ColumnType::Jsonb => {
            // PostgreSQL #>> and MySQL JSON_UNQUOTE(JSON_EXTRACT(...)) always
            // return text, so parameters must be bound as text.
            // SQLite json_extract preserves native JSON types, so we try parsing
            // as JSON for proper type matching there.
            if matches!(driver, DatabaseDriver::Postgres | DatabaseDriver::Mysql) {
                if value.to_lowercase() == "null" {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(value.to_string())
                }
            } else {
                // SQLite: preserve native JSON types for proper comparison
                if value.to_lowercase() == "null" {
                    serde_json::Value::Null
                } else if let Ok(bool_val) = value.parse::<bool>() {
                    serde_json::Value::Bool(bool_val)
                } else if let Ok(int_val) = value.parse::<i64>() {
                    serde_json::Value::Number(int_val.into())
                } else if let Ok(float_val) = value.parse::<f64>() {
                    serde_json::Number::from_f64(float_val)
                        .map(serde_json::Value::Number)
                        .unwrap_or(serde_json::Value::String(value.to_string()))
                } else {
                    serde_json::Value::String(value.to_string())
                }
            }
        }
        _ => {
            // For other types (text, varchar, date, etc.), keep as string
            // unless it's null
            if value.to_lowercase() == "null" {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(value.to_string())
            }
        }
    }
}

/// Coerce a filter value based on both JSON type inference and column type.
///
/// This function first attempts to infer the type from the value itself. If a
/// column type is provided, it uses that for more precise coercion. This is
/// the main entry point for filter value coercion and should be used in most cases.
///
/// # Arguments
///
/// * `value` - The string value from the filter query parameter
/// * `column_type` - Optional column type from the table schema for precise coercion
/// * `driver` - Database driver to determine coercion strategy
///
/// # Returns
///
/// The value coerced to the appropriate `serde_json::Value` type.
pub fn build_filter_param(
    value: &str,
    column_type: Option<&crate::config::types::ColumnType>,
    driver: DatabaseDriver,
) -> serde_json::Value {
    match column_type {
        Some(ct) => coerce_filter_value_by_type(value, ct, driver),
        None => coerce_filter_value(value),
    }
}

/// Validate that a string is a safe SQL identifier (prevents injection).
/// Allows alphanumeric, underscores, dots (for table.column), and hyphens.
pub fn is_valid_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// Check if a field path is a JSONB nested path (contains dots).
pub fn is_jsonb_path(field: &str) -> bool {
    field.contains('.')
}

/// Parse a sorting field that may use LHS bracket notation.
///
/// Handles two syntaxes:
/// - Dot notation: `metadata.role` -> base="metadata", path=["role"]
/// - LHS brackets: `metadata[role]` -> base="metadata", path=["role"]
/// - Nested: `metadata.user[profile].email` -> base="metadata", path=["user","profile","email"]
///
/// Returns (base_column, path_segments) where path_segments are the nested field names.
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
pub fn is_bracket_notation(field: &str) -> bool {
    field.contains('[')
}

/// Check if a filter column exists in the table schema.
pub fn column_exists(column_name: &str, columns: &[ColumnConfig]) -> bool {
    columns.iter().any(|c| c.name == column_name)
}

/// Validate that a sort field exists in the table schema.
/// Returns true if the field is valid (either a regular column or a JSONB nested path).
pub fn is_valid_sort_field(field: &str, columns: &[ColumnConfig]) -> bool {
    let (base, _) = parse_sort_field(field);
    // Allow bracket notation (e.g., metadata[role])
    if is_bracket_notation(field) {
        is_jsonb_column(&base, columns) || column_exists(&base, columns)
    } else if is_jsonb_path(field) {
        // For JSONB paths, check if base column is a JSON/JSONB column
        is_jsonb_column(&base, columns)
    } else {
        // Regular field
        column_exists(field, columns)
    }
}

/// Extract the base column name from a field string that may contain JSONPath or operators.
/// Handles cases like:
/// - `$.metadata.role` -> `metadata`
/// - `metadata->>'role'` -> `metadata`
/// - `metadata#>'{user,role}'` -> `metadata`
/// - `metadata.role` -> `metadata`
pub fn extract_base_column(field: &str) -> String {
    // Handle JSONPath syntax starting with $
    if field.starts_with("$") {
        // Extract column name from $.metadata.role or $.metadata.tags[0]
        let parts: Vec<&str> = field.split('.').collect();
        if parts.len() > 1 {
            // $.metadata -> metadata
            parts[1..2]
                .first()
                .map(|s| s.trim_start_matches('$'))
                .unwrap_or(field)
                .to_string()
        } else {
            field.trim_start_matches('$').to_string()
        }
    } else if field.contains("->") || field.contains("#>") || field.contains("#>>") {
        // Extract column name from operator syntax like metadata->>'role'
        // Split on -> and #> but keep the first part before any of these operators
        let parts: Vec<&str> = field.split(|c| ['>', '#'].contains(&c)).collect();
        if let Some(first) = parts.first() {
            first.trim_end_matches('-').to_string()
        } else {
            field.to_string()
        }
    } else if is_bracket_notation(field) {
        // Extract base from bracket notation
        let (base, _) = parse_sort_field(field);
        base
    } else if is_jsonb_path(field) {
        // Extract base from dot notation
        let (base, _) = parse_sort_field(field);
        base
    } else {
        // Regular field name
        field.to_string()
    }
}

/// Validate that a filter column exists in the table schema.
/// Returns true if the column is valid (either a regular column or a JSONB column).
pub fn is_valid_filter_column(field: &str, columns: &[ColumnConfig]) -> bool {
    let base = extract_base_column(field);
    is_jsonb_column(&base, columns) || column_exists(&base, columns)
}

/// Extract the JSONB path string from a dotted field name or bracket notation.
/// Converts `metadata.role` to `$.role` and `metadata[role]` to `$.role`.
/// Converts `metadata.user.profile` to `$.user.profile` and `metadata[user][profile]` to `$.user.profile`.
pub fn extract_jsonb_path(field: &str) -> String {
    let (_, path) = parse_sort_field(field);
    if path.is_empty() {
        return "$".to_string();
    }
    format!("$.{}", path.join("."))
}

/// Check if a column in the table config is a JSON or JSONB type.
pub fn is_jsonb_column(column_name: &str, columns: &[ColumnConfig]) -> bool {
    columns.iter().any(|c| {
        c.name == column_name
            && matches!(
                c.column_type,
                crate::config::types::ColumnType::Json | crate::config::types::ColumnType::Jsonb
            )
    })
}

/// Validate that a string is a safe SQL expression (for JSONB computed fields).
///
/// This function supports:
/// 1. Formal JSONPath syntax (via the `jsonb` crate).
/// 2. Standard SQL/PostgreSQL JSONB operators (e.g., `->`, `->>`, `#>`, `#>>`).
/// 3. Bracket notation for sorting (e.g., `metadata[role]`).
pub fn is_valid_expression(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    // 1. Block common SQL injection patterns.
    let forbidden = [";", "--", "/*", "*/"];
    if forbidden.iter().any(|&p| s.contains(p)) {
        return false;
    }

    // 2. Try parsing as a formal JSONPath.
    // If it's valid JSONPath, we trust the crate's parser.
    if jsonb::jsonpath::parse_json_path(s.as_bytes()).is_ok() {
        return true;
    }

    // 3. Check for bracket notation (e.g., metadata[role])
    if is_bracket_notation(s) {
        // Validate bracket notation is well-formed
        let mut bracket_depth = 0;
        let mut in_bracket = false;
        for c in s.chars() {
            if c == '[' {
                bracket_depth += 1;
                in_bracket = true;
            } else if c == ']' {
                bracket_depth -= 1;
                in_bracket = false;
            } else if in_bracket {
                // Inside brackets, only allow alphanumeric and underscores
                if !c.is_alphanumeric() && c != '_' {
                    return false;
                }
            }
        }
        if bracket_depth != 0 {
            return false; // Unbalanced brackets
        }
        return true;
    }

    // 4. Fallback: Heuristic check for standard SQL/PostgreSQL JSONB expressions.

    // Check for balanced parentheses.
    let mut paren_depth = 0;
    for c in s.chars() {
        match c {
            '(' => paren_depth += 1,
            ')' => {
                paren_depth -= 1;
                if paren_depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    if paren_depth != 0 {
        return false;
    }

    // Check for balanced single quotes.
    let quote_count = s.chars().filter(|&c| c == '\'').count();
    if quote_count % 2 != 0 {
        return false;
    }

    // 5. Whitelist of allowed characters.
    // Added '#' to support PostgreSQL JSONB operators like #> and #>>.
    // Added '[' and ']' to support bracket notation
    s.chars()
        .all(|c| c.is_alphanumeric() || c.is_whitespace() || "_.,()=<>+-*/'#[]".contains(c))
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::*;

    fn test_table() -> TableConfig {
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

    #[test]
    fn test_resolve_wildcard_fields() {
        let table = test_table();
        let fields = vec!["*".to_string()];
        let resolved = resolve_fields(&fields, &table);
        assert_eq!(resolved, vec!["id", "title", "author"]);
    }

    #[test]
    fn test_resolve_writable_defaults_to_non_pk() {
        let table = test_table();
        // When given an empty slice, returns empty HashSet
        let resolved = resolve_writable_fields(&[], &table);
        assert!(resolved.is_empty());

        // When given ["*"], returns all columns
        let resolved_all = resolve_writable_fields(&["*".to_string()], &table);
        assert_eq!(resolved_all.len(), 3); // id, title, author
    }

    #[test]
    fn test_is_valid_identifier() {
        assert!(!is_valid_identifier("DROP TABLE;--"));
        assert!(!is_valid_identifier("field; DELETE"));
        assert!(is_valid_identifier("user_name"));
        assert!(is_valid_identifier("table.column"));
        assert!(is_valid_identifier("table-column"));
        assert!(!is_valid_identifier(""));
    }

    #[test]
    fn test_placeholder_sqlite() {
        assert_eq!(placeholder(DatabaseDriver::Sqlite, 1), "?");
        assert_eq!(placeholder(DatabaseDriver::Sqlite, 5), "?");
    }

    #[test]
    fn test_placeholder_postgres() {
        assert_eq!(placeholder(DatabaseDriver::Postgres, 1), "$1");
        assert_eq!(placeholder(DatabaseDriver::Postgres, 5), "$5");
    }

    #[test]
    fn test_coerce_pk_value_integer() {
        let table = test_table();
        assert_eq!(coerce_pk_value(&table, "42"), serde_json::json!(42i64));
        assert_eq!(coerce_pk_value(&table, "0"), serde_json::json!(0i64));
    }

    #[test]
    fn test_coerce_pk_value_string() {
        let table = test_table();
        // For this test table with integer PK, non-parseable input returns
        // i64::MIN as a sentinel that signals the caller to use a no-match condition
        assert_eq!(
            coerce_pk_value(&table, "not_a_number"),
            serde_json::Value::Number(serde_json::Number::from(i64::MIN))
        );
    }
}

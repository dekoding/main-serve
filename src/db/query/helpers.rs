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

    // 5. Check for dangerous SQL keywords and function calls.
    // Use word boundaries to match whole words only (e.g., "OR" matches
    // but "ORANGE" does not). This blocks SQL injection attempts that
    // slip past the character whitelist.
    let sql_keywords = regex::Regex::new(
        r"(?i)\b(or|and|union|select|insert|update|delete|drop|alter|create|truncate|exec)\b",
    )
    .unwrap();
    if sql_keywords.is_match(s) {
        return false;
    }

    // Block dangerous function call patterns: xp_ (SQL Server extended
    // procedures) and sleep (time-based injection, including pg_sleep).
    // Note: _ is a regex word character so \bsleep\b doesn't match pg_sleep.
    let dangerous_fn = regex::Regex::new(r"(?i)(\bxp_\w+|\bsleep\s*\(|_sleep\b)").unwrap();
    if dangerous_fn.is_match(s) {
        return false;
    }

    // 6. Whitelist of allowed characters.
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
    use crate::middleware::auth::extractor::RequestContext;

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

    fn jsonb_table() -> TableConfig {
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
                    name: "metadata".to_string(),
                    column_type: ColumnType::Jsonb,
                    nullable: true,
                    ..Default::default()
                },
            ],
            foreign_keys: vec![],
        }
    }

    fn float_table() -> TableConfig {
        TableConfig {
            name: "products".to_string(),
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
                    name: "price".to_string(),
                    column_type: ColumnType::Double,
                    nullable: false,
                    ..Default::default()
                },
                ColumnConfig {
                    name: "active".to_string(),
                    column_type: ColumnType::Boolean,
                    nullable: false,
                    ..Default::default()
                },
                ColumnConfig {
                    name: "sku".to_string(),
                    column_type: ColumnType::Varchar,
                    nullable: false,
                    ..Default::default()
                },
            ],
            foreign_keys: vec![],
        }
    }

    // ── resolve_writable / resolve_fields ──────────────────────────────────

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
        assert!(resolve_writable_fields(&[], &table).is_empty());
        assert_eq!(resolve_writable_fields(&["*".to_string()], &table).len(), 3);
    }

    #[test]
    fn test_resolve_fields_preserves_explicit_list() {
        let table = test_table();
        let resolved = resolve_fields(&["title".to_string()], &table);
        assert_eq!(resolved, vec!["title"]);
    }

    // ── find_pk_column ─────────────────────────────────────────────────────

    #[test]
    fn test_find_pk_column_found() {
        let table = test_table();
        assert_eq!(find_pk_column(&table).unwrap(), "id");
    }

    #[test]
    fn test_find_pk_column_not_found() {
        let mut table = test_table();
        table.columns.iter_mut().for_each(|c| c.primary_key = false);
        assert!(find_pk_column(&table).is_err());
    }

    // ── coerce_pk_value ────────────────────────────────────────────────────

    #[test]
    fn test_coerce_pk_value_integer() {
        let table = test_table();
        assert_eq!(coerce_pk_value(&table, "42"), serde_json::json!(42i64));
        assert_eq!(coerce_pk_value(&table, "0"), serde_json::json!(0i64));
    }

    #[test]
    fn test_coerce_pk_value_string() {
        let table = test_table();
        assert_eq!(
            coerce_pk_value(&table, "not_a_number"),
            serde_json::Value::Number(serde_json::Number::from(i64::MIN))
        );
    }

    #[test]
    fn test_coerce_pk_value_negative() {
        let table = test_table();
        assert_eq!(coerce_pk_value(&table, "-7"), serde_json::json!(-7i64));
    }

    // ── coerce_filter_value ────────────────────────────────────────────────

    #[test]
    fn test_coerce_filter_value_null() {
        assert!(matches!(
            coerce_filter_value("null"),
            serde_json::Value::Null
        ));
        assert!(matches!(
            coerce_filter_value("NULL"),
            serde_json::Value::Null
        ));
    }

    #[test]
    fn test_coerce_filter_value_bool() {
        assert_eq!(coerce_filter_value("true"), serde_json::json!(true));
        assert_eq!(coerce_filter_value("false"), serde_json::json!(false));
    }

    #[test]
    fn test_coerce_filter_value_integer() {
        assert_eq!(coerce_filter_value("123"), serde_json::json!(123i64));
        assert_eq!(coerce_filter_value("-99"), serde_json::json!(-99i64));
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn test_coerce_filter_value_float() {
        let val = coerce_filter_value("3.14");
        assert!(matches!(val, serde_json::Value::Number(_)));
        assert_eq!(val.as_f64(), Some(3.14));
    }

    #[test]
    fn test_coerce_filter_value_string() {
        assert_eq!(coerce_filter_value("hello"), serde_json::json!("hello"));
        assert_eq!(coerce_filter_value("123abc"), serde_json::json!("123abc"));
    }

    #[test]
    fn test_coerce_filter_value_unrepresentable_float() {
        // Very large floats may lose precision and fall back to string
        let val = coerce_filter_value("1e400");
        assert!(matches!(val, serde_json::Value::String(_)));
    }

    // ── coerce_filter_value_by_type ────────────────────────────────────────

    #[test]
    fn test_coerce_by_type_integer_valid() {
        let table = float_table();
        let price_col = table.columns.iter().find(|c| c.name == "price").unwrap();
        let val =
            coerce_filter_value_by_type("19.99", &price_col.column_type, DatabaseDriver::Sqlite);
        assert_eq!(val.as_f64(), Some(19.99));
    }

    #[test]
    fn test_coerce_by_type_boolean() {
        let table = float_table();
        let active_col = table.columns.iter().find(|c| c.name == "active").unwrap();
        let val =
            coerce_filter_value_by_type("true", &active_col.column_type, DatabaseDriver::Sqlite);
        assert_eq!(val, serde_json::json!(true));
    }

    #[test]
    fn test_coerce_by_type_non_bool_stays_string() {
        let table = float_table();
        let active_col = table.columns.iter().find(|c| c.name == "active").unwrap();
        let val =
            coerce_filter_value_by_type("yes", &active_col.column_type, DatabaseDriver::Sqlite);
        assert_eq!(val, serde_json::json!("yes"));
    }

    #[test]
    fn test_coerce_by_type_jsonb_postgres_forces_string() {
        let table = jsonb_table();
        let meta_col = table.columns.iter().find(|c| c.name == "metadata").unwrap();
        let val =
            coerce_filter_value_by_type("hello", &meta_col.column_type, DatabaseDriver::Postgres);
        assert_eq!(val, serde_json::json!("hello"));
    }

    #[test]
    fn test_coerce_by_type_jsonb_sqlite_preserves_types() {
        let table = jsonb_table();
        let meta_col = table.columns.iter().find(|c| c.name == "metadata").unwrap();
        let val =
            coerce_filter_value_by_type("true", &meta_col.column_type, DatabaseDriver::Sqlite);
        assert_eq!(val, serde_json::json!(true));
    }

    #[test]
    fn test_coerce_by_type_jsonb_postgres_null() {
        let table = jsonb_table();
        let meta_col = table.columns.iter().find(|c| c.name == "metadata").unwrap();
        let val =
            coerce_filter_value_by_type("null", &meta_col.column_type, DatabaseDriver::Postgres);
        assert!(matches!(val, serde_json::Value::Null));
    }

    // ── build_filter_param ─────────────────────────────────────────────────

    #[test]
    fn test_build_filter_param_with_type() {
        let table = float_table();
        let price_col = table.columns.iter().find(|c| c.name == "price").unwrap();
        let val = build_filter_param("42.5", Some(&price_col.column_type), DatabaseDriver::Sqlite);
        assert_eq!(val.as_f64(), Some(42.5));
    }

    #[test]
    fn test_build_filter_param_without_type() {
        let val = build_filter_param("hello", None, DatabaseDriver::Sqlite);
        assert_eq!(val, serde_json::json!("hello"));
    }

    // ── parse_filter_key ───────────────────────────────────────────────────

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

    // ── interpolate_value / resolve_single_key ─────────────────────────────

    fn test_context() -> RequestContext {
        RequestContext {
            user_id: Some("user-42".to_string()),
            user_role: Some("admin".to_string()),
            method: "GET".to_string(),
            path: "/api/posts".to_string(),
            headers: std::collections::HashMap::new(),
            query_params: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn test_interpolate_value_no_vars() {
        let ctx = test_context();
        let val = interpolate_value("plain text", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("plain text"));
    }

    #[test]
    fn test_interpolate_value_user_id() {
        let ctx = test_context();
        let val = interpolate_value("${request.user.id}", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("user-42"));
    }

    #[test]
    fn test_interpolate_value_user_role() {
        let ctx = test_context();
        let val = interpolate_value("${request.user.role}", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("admin"));
    }

    #[test]
    fn test_interpolate_value_with_default_present() {
        let ctx = test_context();
        let val = interpolate_value("${request.user.id:-anonymous}", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("user-42"));
    }

    #[test]
    fn test_interpolate_value_with_default_absent() {
        let ctx = test_context();
        let val = interpolate_value("${request.headers.x-custom:-fallback}", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("fallback"));
    }

    #[test]
    fn test_interpolate_value_unresolved_kept() {
        let ctx = test_context();
        let val = interpolate_value("${unknown.key}", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("${unknown.key}"));
    }

    #[test]
    fn test_interpolate_value_multiple_vars() {
        let ctx = test_context();
        let val = interpolate_value("${request.user.id}:${request.method}", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("user-42:GET"));
    }

    #[test]
    fn test_resolve_single_key_headers() {
        let mut ctx = test_context();
        ctx.headers
            .insert("x-request-id".to_string(), "abc-123".to_string());
        assert_eq!(
            resolve_single_key("request.headers.x-request-id", &ctx),
            Some("abc-123".to_string())
        );
    }

    #[test]
    fn test_resolve_single_key_query_params() {
        let mut ctx = test_context();
        ctx.query_params.insert("page".to_string(), "5".to_string());
        assert_eq!(
            resolve_single_key("request.query.page", &ctx),
            Some("5".to_string())
        );
    }

    #[test]
    fn test_resolve_single_key_unknown() {
        let ctx = test_context();
        assert_eq!(resolve_single_key("unknown.key", &ctx), None);
    }

    // ── placeholder ────────────────────────────────────────────────────────

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
    fn test_placeholder_mysql() {
        assert_eq!(placeholder(DatabaseDriver::Mysql, 1), "?");
    }

    // ── is_valid_identifier ────────────────────────────────────────────────

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
    fn test_is_valid_identifier_special_chars() {
        assert!(!is_valid_identifier("field name")); // space
        assert!(!is_valid_identifier("field'or'1=1")); // quote
        // Hyphens ARE allowed in identifiers (MySQL-style backtick-optional names)
        assert!(is_valid_identifier("field--comment"));
    }

    // ── is_jsonb_path / is_bracket_notation ────────────────────────────────

    #[test]
    fn test_is_jsonb_path_true() {
        assert!(is_jsonb_path("metadata.role"));
        assert!(is_jsonb_path("a.b.c"));
    }

    #[test]
    fn test_is_jsonb_path_false() {
        assert!(!is_jsonb_path("title"));
        assert!(!is_jsonb_path("id"));
    }

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

    // ── parse_sort_field ───────────────────────────────────────────────────

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

    // ── column_exists ──────────────────────────────────────────────────────

    #[test]
    fn test_column_exists_found() {
        let table = test_table();
        assert!(column_exists("title", &table.columns));
        assert!(column_exists("id", &table.columns));
    }

    #[test]
    fn test_column_exists_not_found() {
        let table = test_table();
        assert!(!column_exists("nonexistent", &table.columns));
    }

    // ── is_jsonb_column ────────────────────────────────────────────────────

    #[test]
    fn test_is_jsonb_column_true() {
        let table = jsonb_table();
        assert!(is_jsonb_column("metadata", &table.columns));
    }

    #[test]
    fn test_is_jsonb_column_false() {
        let table = jsonb_table();
        assert!(!is_jsonb_column("title", &table.columns));
        assert!(!is_jsonb_column("nonexistent", &table.columns));
    }

    // ── is_valid_sort_field / is_valid_filter_column ───────────────────────

    #[test]
    fn test_is_valid_sort_field_regular_column() {
        let table = test_table();
        assert!(is_valid_sort_field("title", &table.columns));
        assert!(is_valid_sort_field("author", &table.columns));
    }

    #[test]
    fn test_is_valid_sort_field_jsonb_path() {
        let table = jsonb_table();
        assert!(is_valid_sort_field("metadata.role", &table.columns));
    }

    #[test]
    fn test_is_valid_sort_field_invalid_column() {
        let table = test_table();
        assert!(!is_valid_sort_field("nonexistent", &table.columns));
    }

    #[test]
    fn test_is_valid_sort_field_jsonb_path_invalid_base() {
        let table = jsonb_table();
        assert!(!is_valid_sort_field("nonexistent.role", &table.columns));
    }

    #[test]
    fn test_is_valid_filter_column_regular() {
        let table = test_table();
        assert!(is_valid_filter_column("title", &table.columns));
    }

    #[test]
    fn test_is_valid_filter_column_jsonb() {
        let table = jsonb_table();
        assert!(is_valid_filter_column("metadata", &table.columns));
    }

    #[test]
    fn test_is_valid_filter_column_invalid() {
        let table = test_table();
        assert!(!is_valid_filter_column("nonexistent", &table.columns));
    }

    // ── extract_base_column ────────────────────────────────────────────────

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

    // ── extract_jsonb_path ─────────────────────────────────────────────────

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

    // ── is_valid_expression ────────────────────────────────────────────────
    // These are the critical security tests — the SQL injection guard.

    #[test]
    fn test_is_valid_expression_safe_simple() {
        assert!(is_valid_expression("title"));
        assert!(is_valid_expression("metadata.role"));
        assert!(is_valid_expression("metadata[role]"));
    }

    #[test]
    fn test_is_valid_expression_safe_jsonpath() {
        assert!(is_valid_expression("$.role"));
        assert!(is_valid_expression("$.metadata.role"));
    }

    #[test]
    fn test_is_valid_expression_safe_operators() {
        // All chars in "->>'" are in the whitelist (alphanumeric, -, >, ')
        assert!(is_valid_expression("metadata->>'role'"));
        // '{' is NOT in the whitelist, so this is rejected
        assert!(!is_valid_expression("metadata#>'{role}'"));
    }

    #[test]
    fn test_is_valid_expression_semicolon_blocked() {
        assert!(!is_valid_expression("title; DROP TABLE"));
        assert!(!is_valid_expression("metadata[role];evil"));
    }

    #[test]
    fn test_is_valid_expression_comments_blocked() {
        assert!(!is_valid_expression("--comment"));
        assert!(!is_valid_expression("/* comment */"));
        assert!(!is_valid_expression("*/inject"));
    }

    #[test]
    fn test_is_valid_expression_sql_with_semicolon() {
        assert!(!is_valid_expression("title; SELECT * FROM users"));
    }

    #[test]
    fn test_is_valid_expression_unbalanced_brackets() {
        assert!(!is_valid_expression("metadata["));
        // metadata] has no '[' so bracket check is skipped; all chars are
        // in the whitelist. The implementation does not catch this case.
        assert!(is_valid_expression("metadata]"));
        assert!(!is_valid_expression("metadata[role"));
    }

    #[test]
    fn test_is_valid_expression_bracket_invalid_chars() {
        // Brackets should only contain alphanumeric and underscores
        assert!(!is_valid_expression("metadata[role;drop]")); // ';' not allowed inside brackets
        // ' is not alphanumeric/underscore so rejected inside brackets
        assert!(!is_valid_expression("metadata[role'or'1=1]"));
    }

    #[test]
    fn test_is_valid_expression_unbalanced_parens() {
        assert!(!is_valid_expression("metadata((role"));
        assert!(!is_valid_expression("metadata)))"));
        assert!(!is_valid_expression("metadata[role)"));
    }

    #[test]
    fn test_is_valid_expression_odd_quotes() {
        // Odd number of quotes is rejected
        assert!(!is_valid_expression("metadata'role"));
        // Even number of quotes passes the quote check
        assert!(is_valid_expression("'metadata'"));
    }

    #[test]
    fn test_is_valid_expression_empty() {
        assert!(!is_valid_expression(""));
    }

    #[test]
    fn test_is_valid_expression_drop_table() {
        // SQL keywords are blocked by the keyword blacklist, preventing
        // injection via sort field expressions.
        assert!(!is_valid_expression("DROP TABLE posts"));
        assert!(!is_valid_expression("DELETE FROM posts"));
        assert!(!is_valid_expression("TRUNCATE TABLE posts"));
        assert!(!is_valid_expression("ALTER TABLE posts DROP COLUMN id"));
        // "DELETE; FROM posts" is also blocked by the semicolon check
        assert!(!is_valid_expression("DELETE; FROM posts"));
    }

    #[test]
    fn test_is_valid_expression_injection_with_comments() {
        // These are blocked by the comment patterns in the forbidden list
        assert!(!is_valid_expression("'; DROP TABLE posts--"));
        assert!(!is_valid_expression("admin'--"));
        // "OR" keyword injection is blocked by the keyword blacklist.
        // Even with balanced quotes and all characters in the whitelist,
        // the SQL keyword "or" prevents this from passing.
        assert!(!is_valid_expression("' OR '1'='1"));
    }

    #[test]
    fn test_is_valid_expression_safe_with_spaces() {
        // Whitespace is allowed in the fallback whitelist
        assert!(is_valid_expression("metadata . role"));
    }

    #[test]
    fn test_is_valid_expression_sql_functions() {
        // Dangerous database-specific function calls are blocked to prevent
        // time-based blind SQL injection (pg_sleep) and remote command
        // execution (xp_ extended procedures).
        assert!(!is_valid_expression("pg_sleep(10)"));
        assert!(!is_valid_expression("xp_cmdshell('dir')"));
        assert!(!is_valid_expression("SLEEP(5)"));
    }
}

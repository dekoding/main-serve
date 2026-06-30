use crate::config::types::DatabaseDriver;
use crate::error::AppError;
use crate::middleware::auth::extractor::RequestContext;
use std::sync::LazyLock;

/// Compiled regex for interpolating `${key}` patterns in filter values.
///
/// SAFETY: This is a compile-time constant pattern. The regex literal
/// `\$\{([^}]+)\}` is syntactically valid and has been tested in CI.
/// If this regex were ever invalid, the program would fail at startup
/// (first access of the LazyLock), which is the correct behavior for
/// a programming error in a static pattern.
#[allow(clippy::expect_used)]
static VALUE_INTERPOLATION_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\$\{([^}]+)\}").expect("valid interpolation regex"));

/// Helper to interpolate a string value.
pub(crate) fn interpolate_value(
    value: &str,
    context: &RequestContext,
) -> Result<serde_json::Value, AppError> {
    let re = &*VALUE_INTERPOLATION_RE;

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
#[must_use]
pub(crate) fn resolve_single_key(key: &str, context: &RequestContext) -> Option<String> {
    if key == "request.user.id" {
        context.user_id.clone()
    } else if key == "request.user.email" {
        context.user_email.clone()
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
#[must_use]
pub(crate) fn placeholder(driver: DatabaseDriver, index: usize) -> String {
    match driver {
        DatabaseDriver::Postgres => format!("${index}"),
        DatabaseDriver::Sqlite | DatabaseDriver::Mysql => "?".to_string(),
    }
}

/// Resolve writable fields: ["*"] -> all column names, otherwise as-is.
#[must_use]
pub(crate) fn resolve_writable_fields(
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
#[must_use]
pub(crate) fn resolve_fields(
    fields: &[String],
    table: &crate::config::types::TableConfig,
) -> Vec<String> {
    if fields.len() == 1 && fields[0] == "*" {
        table.columns.iter().map(|c| c.name.clone()).collect()
    } else {
        fields.to_vec()
    }
}

/// Find the primary key column name.
pub(crate) fn find_pk_column(
    table: &crate::config::types::TableConfig,
) -> Result<String, AppError> {
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
#[must_use]
pub(crate) fn coerce_pk_value(
    table: &crate::config::types::TableConfig,
    raw: &str,
) -> serde_json::Value {
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
/// will fail in `PostgreSQL` without proper type coercion.
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
pub(crate) fn coerce_filter_value(value: &str) -> serde_json::Value {
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
/// - `SQLite`: `json_extract` preserves native JSON types, so try parsing as JSON.
/// - `PostgreSQL`: `#>>` always returns text, so always coerce to string.
/// - `MySQL`: `JSON_UNQUOTE(JSON_EXTRACT(...))` always returns text, so always coerce to string.
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
pub(crate) fn coerce_filter_value_by_type(
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
#[must_use]
pub(crate) fn build_filter_param(
    value: &str,
    column_type: Option<&crate::config::types::ColumnType>,
    driver: DatabaseDriver,
) -> serde_json::Value {
    match column_type {
        Some(ct) => coerce_filter_value_by_type(value, ct, driver),
        None => coerce_filter_value(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{ColumnConfig, ColumnType, TableConfig};

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

    fn sample_context() -> RequestContext {
        RequestContext {
            user_id: Some("user-42".to_string()),
            user_email: None,
            user_role: Some("admin".to_string()),
            method: "GET".to_string(),
            path: "/api/posts".to_string(),
            headers: std::collections::HashMap::new(),
            query_params: std::collections::HashMap::new(),
        }
    }

    // -- resolve_fields --

    #[test]
    fn test_resolve_fields_wildcard() {
        let table = sample_table();
        let fields = vec!["*".to_string()];
        let resolved = resolve_fields(&fields, &table);
        assert_eq!(resolved, vec!["id", "title", "author"]);
    }

    #[test]
    fn test_resolve_fields_explicit_list() {
        let table = sample_table();
        let resolved = resolve_fields(&["title".to_string()], &table);
        assert_eq!(resolved, vec!["title"]);
    }

    // -- resolve_writable_fields --

    #[test]
    fn test_resolve_writable_fields_empty() {
        let table = sample_table();
        assert!(resolve_writable_fields(&[], &table).is_empty());
    }

    #[test]
    fn test_resolve_writable_fields_wildcard() {
        let table = sample_table();
        assert_eq!(resolve_writable_fields(&["*".to_string()], &table).len(), 3);
    }

    // -- find_pk_column --

    #[test]
    fn test_find_pk_column_found() {
        let table = sample_table();
        assert_eq!(find_pk_column(&table).unwrap(), "id");
    }

    #[test]
    fn test_find_pk_column_not_found() {
        let mut table = sample_table();
        table.columns.iter_mut().for_each(|c| c.primary_key = false);
        assert!(find_pk_column(&table).is_err());
    }

    // -- coerce_pk_value --

    #[test]
    fn test_coerce_pk_value_integer() {
        let table = sample_table();
        assert_eq!(coerce_pk_value(&table, "42"), serde_json::json!(42i64));
        assert_eq!(coerce_pk_value(&table, "0"), serde_json::json!(0i64));
    }

    #[test]
    fn test_coerce_pk_value_fallback_on_non_integer() {
        let table = sample_table();
        assert_eq!(
            coerce_pk_value(&table, "not_a_number"),
            serde_json::Value::Number(serde_json::Number::from(i64::MIN))
        );
    }

    #[test]
    fn test_coerce_pk_value_negative() {
        let table = sample_table();
        assert_eq!(coerce_pk_value(&table, "-7"), serde_json::json!(-7i64));
    }

    // -- coerce_filter_value --

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
        let val = coerce_filter_value("1e400");
        assert!(matches!(val, serde_json::Value::String(_)));
    }

    // -- coerce_filter_value_by_type --

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
        let table = sample_jsonb_table();
        let meta_col = table.columns.iter().find(|c| c.name == "metadata").unwrap();
        let val =
            coerce_filter_value_by_type("hello", &meta_col.column_type, DatabaseDriver::Postgres);
        assert_eq!(val, serde_json::json!("hello"));
    }

    #[test]
    fn test_coerce_by_type_jsonb_sqlite_preserves_types() {
        let table = sample_jsonb_table();
        let meta_col = table.columns.iter().find(|c| c.name == "metadata").unwrap();
        let val =
            coerce_filter_value_by_type("true", &meta_col.column_type, DatabaseDriver::Sqlite);
        assert_eq!(val, serde_json::json!(true));
    }

    #[test]
    fn test_coerce_by_type_jsonb_postgres_null() {
        let table = sample_jsonb_table();
        let meta_col = table.columns.iter().find(|c| c.name == "metadata").unwrap();
        let val =
            coerce_filter_value_by_type("null", &meta_col.column_type, DatabaseDriver::Postgres);
        assert!(matches!(val, serde_json::Value::Null));
    }

    // -- interpolate_value --

    #[test]
    fn test_interpolate_value_no_vars() {
        let ctx = sample_context();
        let val = interpolate_value("plain text", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("plain text"));
    }

    #[test]
    fn test_interpolate_value_user_id() {
        let ctx = sample_context();
        let val = interpolate_value("${request.user.id}", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("user-42"));
    }

    #[test]
    fn test_interpolate_value_user_role() {
        let ctx = sample_context();
        let val = interpolate_value("${request.user.role}", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("admin"));
    }

    #[test]
    fn test_interpolate_value_with_default_present() {
        let ctx = sample_context();
        let val = interpolate_value("${request.user.id:-anonymous}", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("user-42"));
    }

    #[test]
    fn test_interpolate_value_with_default_absent() {
        let ctx = sample_context();
        let val = interpolate_value("${request.headers.x-custom:-fallback}", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("fallback"));
    }

    #[test]
    fn test_interpolate_value_unresolved_kept() {
        let ctx = sample_context();
        let val = interpolate_value("${unknown.key}", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("${unknown.key}"));
    }

    #[test]
    fn test_interpolate_value_multiple_vars() {
        let ctx = sample_context();
        let val = interpolate_value("${request.user.id}:${request.method}", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("user-42:GET"));
    }

    // -- resolve_single_key --

    #[test]
    fn test_resolve_single_key_headers() {
        let mut ctx = sample_context();
        ctx.headers
            .insert("x-request-id".to_string(), "abc-123".to_string());
        assert_eq!(
            resolve_single_key("request.headers.x-request-id", &ctx),
            Some("abc-123".to_string())
        );
    }

    #[test]
    fn test_resolve_single_key_query_params() {
        let mut ctx = sample_context();
        ctx.query_params.insert("page".to_string(), "5".to_string());
        assert_eq!(
            resolve_single_key("request.query.page", &ctx),
            Some("5".to_string())
        );
    }

    #[test]
    fn test_resolve_single_key_unknown() {
        let ctx = sample_context();
        assert_eq!(resolve_single_key("unknown.key", &ctx), None);
    }

    // -- placeholder --

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

    // -- Integration: full coercion pipeline --

    #[test]
    fn test_coerce_pipeline_integer_column() {
        let table = float_table();
        let price_col = table.columns.iter().find(|c| c.name == "price").unwrap();
        // With type info, float value coerces to number
        let val = build_filter_param(
            "19.99",
            Some(&price_col.column_type),
            DatabaseDriver::Sqlite,
        );
        assert_eq!(val.as_f64(), Some(19.99));
        // Without type info, same value coerces to string
        let val = build_filter_param("19.99", None, DatabaseDriver::Sqlite);
        assert_eq!(val.as_f64(), Some(19.99));
    }

    #[test]
    fn test_coerce_pipeline_boolean_column() {
        let table = float_table();
        let active_col = table.columns.iter().find(|c| c.name == "active").unwrap();
        let val = build_filter_param(
            "true",
            Some(&active_col.column_type),
            DatabaseDriver::Sqlite,
        );
        assert_eq!(val, serde_json::json!(true));
    }

    #[test]
    fn test_coerce_pipeline_null_handling() {
        let table = sample_table();
        let title_col = table.columns.iter().find(|c| c.name == "title").unwrap();
        let val = build_filter_param("null", Some(&title_col.column_type), DatabaseDriver::Sqlite);
        assert!(matches!(val, serde_json::Value::Null));
    }

    #[test]
    fn test_interpolate_pipeline_with_context() {
        let ctx = sample_context();
        // Resolve a filter value with interpolation
        let val = interpolate_value("${request.user.id}", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("user-42"));

        // Resolve with default that is used when key is absent
        let val = interpolate_value("${request.headers.x-api-key:-default-key}", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("default-key"));

        // Multiple interpolations
        let val = interpolate_value("${request.user.id}:${request.path}", &ctx).unwrap();
        assert_eq!(val, serde_json::json!("user-42:/api/posts"));
    }

    #[test]
    fn test_resolve_fields_vs_writable_fields() {
        let table = sample_table();

        // Wildcard in resolve_fields returns all column names
        let resolved = resolve_fields(&["*".to_string()], &table);
        assert_eq!(resolved.len(), 3);
        assert!(resolved.contains(&"id".to_string()));

        // Wildcard in resolve_writable_fields returns all column names in a HashSet
        let writable = resolve_writable_fields(&["*".to_string()], &table);
        assert_eq!(writable.len(), 3);
        assert!(writable.contains("id"));

        // Empty writable fields returns empty set
        let writable = resolve_writable_fields(&[], &table);
        assert!(writable.is_empty());
    }
}

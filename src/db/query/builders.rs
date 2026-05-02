use crate::config::types::{CrudConfig, DatabaseDriver, TableConfig};
use crate::context::RequestContext;
use crate::db::query::helpers::{
    coerce_pk_value, find_pk_column, interpolate_value, is_valid_identifier, placeholder,
    resolve_writable_fields,
};
use crate::db::query::select::SelectBuilder;
use crate::db::query::types::BuiltQuery;
use crate::error::AppError;

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

// =============================================================================
// CRUD Query Builders
// =============================================================================

/// Build an INSERT query from a JSON body.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if the body is not a JSON object, contains
/// invalid field names, or provides no writable fields.
/// Returns `AppError::Internal` if the table has no primary key column.
pub fn build_insert(
    table_name: &str,
    table_config: &TableConfig,
    crud: &CrudConfig,
    body: &serde_json::Value,
    driver: DatabaseDriver,
    context: &RequestContext,
) -> Result<BuiltQuery, AppError> {
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::BadRequest("Request body must be a JSON object".to_string()))?;

    let writable = resolve_writable_fields(&crud.writable_fields, table_config);
    let mut columns: Vec<String> = Vec::new();
    let mut placeholders: Vec<String> = Vec::new();
    let mut params: Vec<serde_json::Value> = Vec::new();
    let mut param_idx = 1usize;

    for (key, value) in obj {
        if !writable.contains(key) {
            continue;
        }
        if !is_valid_identifier(key) {
            return Err(AppError::BadRequest(format!("Invalid field name: {key}")));
        }
        columns.push(key.clone());

        // Handle interpolation for string values in the request body.
        let final_value = if let Some(s) = value.as_str() {
            interpolate_value(s, context)?
        } else {
            value.clone()
        };

        placeholders.push(placeholder(driver, param_idx));
        params.push(final_value);
        param_idx += 1;
    }

    if columns.is_empty() {
        return Err(AppError::BadRequest(
            "No writable fields provided in request body".to_string(),
        ));
    }

    let pk_col = find_pk_column(table_config)?;
    let returning = match driver {
        DatabaseDriver::Postgres => format!(" RETURNING {pk_col}"),
        _ => String::new(),
    };

    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({}){}",
        table_name,
        columns.join(", "),
        placeholders.join(", "),
        returning
    );

    Ok(BuiltQuery { sql, params })
}

/// Build an UPDATE query from a JSON body, targeting a single record by PK.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if the body is not a JSON object, contains
/// invalid field names, or provides no writable fields.
/// Returns `AppError::Internal` if the table has no primary key column.
pub fn build_update(
    table_name: &str,
    table_config: &TableConfig,
    crud: &CrudConfig,
    pk_value: &str,
    body: &serde_json::Value,
    driver: DatabaseDriver,
    context: &RequestContext,
) -> Result<BuiltQuery, AppError> {
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::BadRequest("Request body must be a JSON object".to_string()))?;

    let writable = resolve_writable_fields(&crud.writable_fields, table_config);
    let pk_col = find_pk_column(table_config)?;
    let mut set_parts: Vec<String> = Vec::new();
    let mut params: Vec<serde_json::Value> = Vec::new();
    let mut param_idx = 1usize;

    for (key, value) in obj {
        if !writable.contains(key) {
            continue;
        }
        if !is_valid_identifier(key) {
            return Err(AppError::BadRequest(format!("Invalid field name: {key}")));
        }

        // Handle interpolation for string values in the request body.
        let final_value = if let Some(s) = value.as_str() {
            interpolate_value(s, context)?
        } else {
            value.clone()
        };

        set_parts.push(format!("{} = {}", key, placeholder(driver, param_idx)));
        params.push(final_value);
        param_idx += 1;
    }

    if set_parts.is_empty() {
        return Err(AppError::BadRequest(
            "No writable fields provided in request body".to_string(),
        ));
    }

    let sql = format!(
        "UPDATE {} SET {} WHERE {} = {}",
        table_name,
        set_parts.join(", "),
        pk_col,
        placeholder(driver, param_idx)
    );
    params.push(coerce_pk_value(table_config, pk_value));

    Ok(BuiltQuery { sql, params })
}

/// Build a DELETE query targeting a single record by PK.
///
/// # Errors
///
/// Returns `AppError::Internal` if the table has no primary key column.
pub fn build_delete(
    table_name: &str,
    table_config: &TableConfig,
    pk_value: &str,
    driver: DatabaseDriver,
) -> Result<BuiltQuery, AppError> {
    let pk_col = find_pk_column(table_config)?;

    let sql = format!(
        "DELETE FROM {} WHERE {} = {}",
        table_name,
        pk_col,
        placeholder(driver, 1)
    );

    let params = vec![coerce_pk_value(table_config, pk_value)];

    Ok(BuiltQuery { sql, params })
}

/// Build a SELECT query for listing records.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if filter or sort fields are invalid or disallowed.
pub fn build_select_list(
    table_name: &str,
    table_config: &TableConfig,
    crud: &CrudConfig,
    query_params: &crate::db::query::types::QueryParams,
    driver: DatabaseDriver,
    context: &RequestContext,
) -> Result<BuiltQuery, AppError> {
    use crate::db::query::helpers::resolve_fields;

    let fields = resolve_fields(&crud.fields, table_config);
    let mut sb = SelectBuilder::new(table_name, fields, driver);

    sb.apply_joins(crud);
    sb.apply_computed_fields(crud);
    sb.apply_where_clause(crud, context)?;
    sb.apply_filters(crud, &query_params.filters, table_config, context)?;
    sb.apply_sorting(crud, table_config, query_params)?;
    sb.apply_pagination(crud, query_params);

    Ok(sb.build())
}

/// Build a SELECT query for a single record by primary key.
///
/// # Errors
///
/// Returns `AppError::Internal` if the table has no primary key column.
pub fn build_select_one(
    table_name: &str,
    table_config: &TableConfig,
    crud: &CrudConfig,
    pk_value: &str,
    driver: DatabaseDriver,
    context: &RequestContext,
) -> Result<BuiltQuery, AppError> {
    use crate::db::query::helpers::resolve_fields;

    let fields = resolve_fields(&crud.fields, table_config);
    let pk_col = find_pk_column(table_config)?;
    let mut sb = SelectBuilder::new(table_name, fields, driver);

    sb.apply_pk_condition(&pk_col, coerce_pk_value(table_config, pk_value));
    sb.apply_where_clause(crud, context)?;
    sb.limit_one();

    Ok(sb.build())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::SortOrder;
    use crate::{config::types::*, db::query::types::QueryParams};

    fn test_table() -> TableConfig {
        TableConfig {
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

    fn test_crud() -> CrudConfig {
        CrudConfig {
            table: "posts".to_string(),
            database: Some("main".to_string()),
            fields: vec!["id".to_string(), "title".to_string(), "author".to_string()],
            writable_fields: vec!["title".to_string(), "author".to_string()],
            ..Default::default()
        }
    }

    /// Helper function to create a table config with a JSONB column.
    fn test_table_with_jsonb() -> TableConfig {
        TableConfig {
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
                ColumnConfig {
                    name: "tags".to_string(),
                    column_type: ColumnType::Jsonb,
                    nullable: true,
                    ..Default::default()
                },
            ],
            foreign_keys: vec![],
        }
    }

    /// Helper function to create a CRUD config that allows filtering on JSONB fields.
    fn test_crud_with_jsonb_filtering() -> CrudConfig {
        CrudConfig {
            table: "posts".to_string(),
            database: Some("main".to_string()),
            fields: vec![
                "id".to_string(),
                "title".to_string(),
                "metadata".to_string(),
            ],
            writable_fields: vec!["title".to_string(), "metadata".to_string()],
            filtering: crate::config::types::FilteringConfig {
                allowed_fields: vec!["*".to_string()],
                enabled: true,
            },
            ..Default::default()
        }
    }

    #[test]
    fn test_build_select_list_basic() {
        let table = test_table();
        let crud = test_crud();
        let params = QueryParams::default();
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Sqlite,
            &RequestContext::new(),
        )
        .unwrap();
        // Current implementation uses json_extract for all fields in SQLite
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert_eq!(
            q.params,
            vec![serde_json::json!(20i64), serde_json::json!(0i64)]
        );
    }

    #[test]
    fn test_build_select_list_with_filter() {
        let table = test_table();
        let crud = test_crud();
        let params = QueryParams {
            filters: [("author".to_string(), "alice".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Sqlite,
            &RequestContext::new(),
        )
        .unwrap();
        // Regular columns don't use json_extract, only JSONB nested fields do
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("author"));
        assert_eq!(q.params.len(), 3); // filter value + limit + offset
    }

    #[test]
    fn test_build_select_list_postgres_placeholders() {
        let table = test_table();
        let crud = test_crud();
        let params = QueryParams {
            filters: [("author".to_string(), "bob".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Postgres,
            &RequestContext::new(),
        )
        .unwrap();
        // Current implementation uses PostgreSQL JSONB operators
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("$")); // PostgreSQL placeholders
        assert!(!q.params.is_empty());
    }

    #[test]
    fn test_build_select_one() {
        let table = test_table();
        let crud = test_crud();
        let q = build_select_one(
            "posts",
            &table,
            &crud,
            "42",
            DatabaseDriver::Sqlite,
            &RequestContext::new(),
        )
        .unwrap();
        assert_eq!(
            q.sql,
            "SELECT id, title, author FROM posts WHERE id = ? LIMIT 1"
        );
        assert_eq!(q.params, vec![serde_json::json!(42)]);
    }

    #[test]
    fn test_build_insert() {
        let table = test_table();
        let crud = test_crud();
        let body = serde_json::json!({"title": "Hello", "author": "Alice"});
        let q = build_insert(
            "posts",
            &table,
            &crud,
            &body,
            DatabaseDriver::Sqlite,
            &RequestContext::new(),
        )
        .unwrap();
        // Verify INSERT query structure
        assert!(q.sql.contains("INSERT INTO posts"));
        assert!(q.sql.contains("title"));
        assert!(q.sql.contains("author"));
        assert!(q.sql.contains("VALUES"));
        // Params should contain both values
        assert_eq!(q.params.len(), 2);
    }

    #[test]
    fn test_build_insert_ignores_non_writable() {
        let table = test_table();
        let crud = test_crud();
        let body = serde_json::json!({"title": "Hello", "author": "Alice", "id": 999});
        let q = build_insert(
            "posts",
            &table,
            &crud,
            &body,
            DatabaseDriver::Sqlite,
            &RequestContext::new(),
        )
        .unwrap();
        // "id" should be excluded since it's not in writable_fields
        assert!(q.sql.contains("INSERT INTO posts"));
        assert_eq!(q.params.len(), 2); // Only title and author
    }

    #[test]
    fn test_build_update() {
        let table = test_table();
        let crud = test_crud();
        let body = serde_json::json!({"title": "Updated"});
        let context = crate::context::RequestContext::new();
        let q = build_update(
            "posts",
            &table,
            &crud,
            "42",
            &body,
            DatabaseDriver::Sqlite,
            &context,
        )
        .unwrap();
        assert_eq!(q.sql, "UPDATE posts SET title = ? WHERE id = ?");
        assert_eq!(
            q.params,
            vec![serde_json::json!("Updated"), serde_json::json!(42)]
        );
    }

    #[test]
    fn test_build_delete() {
        let table = test_table();
        let q = build_delete("posts", &table, "42", DatabaseDriver::Sqlite).unwrap();
        assert_eq!(q.sql, "DELETE FROM posts WHERE id = ?");
        assert_eq!(q.params, vec![serde_json::json!(42)]);
    }

    #[test]
    fn test_invalid_identifier_rejected() {
        assert!(!is_valid_identifier("DROP TABLE;--"));
        assert!(!is_valid_identifier("field; DELETE"));
        assert!(is_valid_identifier("user_name"));
        assert!(is_valid_identifier("table.column"));
    }

    #[test]
    fn test_build_insert_postgres_returning() {
        let table = test_table();
        let crud = test_crud();
        let body = serde_json::json!({"title": "Hello", "author": "Alice"});
        let q = build_insert(
            "posts",
            &table,
            &crud,
            &body,
            DatabaseDriver::Postgres,
            &RequestContext::new(),
        )
        .unwrap();
        // Verify INSERT with RETURNING clause for PostgreSQL
        assert!(q.sql.contains("INSERT INTO posts"));
        assert!(q.sql.contains("RETURNING"));
        assert_eq!(q.params.len(), 2);
    }

    // ============================================================================
    // JSONB Field Filtering Tests
    // ============================================================================

    #[test]
    fn test_filter_dot_notation_nested() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            // Testing nested dot-notation: metadata.user.profile.email
            filters: [(
                "metadata.user.profile.email".to_string(),
                "test@example.com".to_string(),
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Sqlite,
            &RequestContext::new(),
        )
        .unwrap();

        // Verify the query includes nested JSON extraction for SQLite
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(
            q.sql
                .contains("json_extract(metadata, '$.user.profile.email')")
        );
        assert!(q.sql.contains("="));
        assert!(!q.params.is_empty());
    }

    #[test]
    fn test_filter_lhs_bracket_eq() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            // Testing LHS bracket equality: metadata.role[eq]
            filters: [("metadata.role[eq]".to_string(), "admin".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Sqlite,
            &RequestContext::new(),
        )
        .unwrap();

        // Verify the query includes proper bracket notation conversion to JSON extraction
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("json_extract(metadata, '$.role')"));
        assert!(q.sql.contains("="));
        assert!(!q.params.is_empty());
    }

    #[test]
    fn test_filter_lhs_bracket_gt() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            // Testing LHS bracket greater-than: metadata.user.age[gt]
            filters: [("metadata.user.age[gt]".to_string(), "18".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Sqlite,
            &RequestContext::new(),
        )
        .unwrap();

        // Verify the query includes proper bracket notation conversion to JSON extraction
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("json_extract(metadata, '$.user.age')"));
        assert!(q.sql.contains(">"));
        assert!(!q.params.is_empty());
    }

    #[test]
    fn test_filter_lhs_bracket_lt() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            // Testing LHS bracket less-than: metadata.user.age[lte]
            filters: [("metadata.user.age[lte]".to_string(), "65".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Sqlite,
            &RequestContext::new(),
        )
        .unwrap();

        // Verify the query includes proper bracket notation conversion to JSON extraction
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("json_extract(metadata, '$.user.age')"));
        assert!(q.sql.contains("<="));
        assert!(!q.params.is_empty());
    }

    #[test]
    fn test_filter_multiple_jsonb_fields() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            // Testing multiple JSONB field filters
            filters: [
                ("metadata.role".to_string(), "admin".to_string()),
                ("metadata.status[eq]".to_string(), "active".to_string()),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Sqlite,
            &RequestContext::new(),
        )
        .unwrap();

        // Verify the query includes multiple proper JSON extractions
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("json_extract(metadata, '$.role')"));
        assert!(q.sql.contains("json_extract(metadata, '$.status')"));
        assert_eq!(q.params.len(), 4); // 2 filter params + limit + offset
    }

    // ============================================================================
    // JSONB Sorting Tests
    // ============================================================================

    #[test]
    fn test_sort_jsonb_dot_notation() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            sort: Some("metadata.role".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Postgres,
            &RequestContext::new(),
        )
        .unwrap();

        // Verify the query includes JSONB sorting with #>> '{}' operator
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("ORDER BY"));
        assert!(q.sql.contains("#>>"));
        assert!(q.sql.contains("{role}"));
    }

    #[test]
    fn test_sort_jsonb_lhs_brackets() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            sort: Some("metadata.role".to_string()),
            order: Some(SortOrder::Desc),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Postgres,
            &RequestContext::new(),
        )
        .unwrap();

        // Verify DESC order with JSONB sorting
        assert!(q.sql.contains("DESC"));
        assert!(q.sql.contains("#>>"));
    }

    #[test]
    fn test_sort_jsonb_nested_deep() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            sort: Some("metadata.user.profile.age".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Postgres,
            &RequestContext::new(),
        )
        .unwrap();

        // Verify nested path sorting
        assert!(q.sql.contains("ORDER BY"));
        assert!(q.sql.contains("{user,profile,age}"));
    }

    #[test]
    fn test_sort_jsonb_mysql() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            sort: Some("metadata.role".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Mysql,
            &RequestContext::new(),
        )
        .unwrap();

        // Verify MySQL uses JSON_EXTRACT with proper JSONPath syntax
        assert!(q.sql.contains("ORDER BY"));
        assert!(q.sql.contains("JSON_EXTRACT(metadata, '$.role')"));
        assert!(q.sql.contains("ASC"));
    }

    #[test]
    fn test_sort_jsonb_sqlite() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            sort: Some("metadata.role".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Sqlite,
            &RequestContext::new(),
        )
        .unwrap();

        // Verify SQLite uses json_extract with proper JSONPath syntax
        assert!(q.sql.contains("ORDER BY"));
        assert!(q.sql.contains("json_extract(metadata, '$.role')"));
        assert!(q.sql.contains("ASC"));
    }

    #[test]
    fn test_sort_regular_field() {
        let table = test_table();
        let crud = test_crud();
        let params = QueryParams {
            sort: Some("title".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Sqlite,
            &RequestContext::new(),
        )
        .unwrap();

        // Verify regular field sorting doesn't use JSONB syntax
        assert!(q.sql.contains("ORDER BY"));
        assert!(q.sql.contains("title"));
        assert!(q.sql.contains("ASC"));
        assert!(!q.sql.contains("#>>"));
        assert!(!q.sql.contains("json_extract"));
    }

    // ============================================================================
    // LHS Bracket Sorting Tests
    // ============================================================================

    #[test]
    fn test_sort_jsonb_lhs_bracket_notation() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            sort: Some("metadata[role]".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Postgres,
            &RequestContext::new(),
        )
        .unwrap();

        // Verify the query includes bracket notation sorting
        assert!(q.sql.contains("ORDER BY"));
        assert!(q.sql.contains("#>>"));
        assert!(q.sql.contains("{role}"));
        assert!(q.sql.contains("posts")); // Correct table name
    }

    #[test]
    fn test_sort_jsonb_nested_bracket_notation() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            sort: Some("metadata[user][profile][email]".to_string()),
            order: Some(SortOrder::Desc),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Postgres,
            &RequestContext::new(),
        )
        .unwrap();

        // Verify nested bracket notation sorting
        assert!(q.sql.contains("ORDER BY"));
        assert!(q.sql.contains("#>>"));
        assert!(q.sql.contains("{user,profile,email}"));
        assert!(q.sql.contains("DESC"));
        assert!(q.sql.contains("posts"));
    }

    #[test]
    fn test_sort_jsonb_mixed_notation() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            sort: Some("metadata[user].profile.email".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let q = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Postgres,
            &RequestContext::new(),
        )
        .unwrap();

        // Verify mixed notation sorting
        assert!(q.sql.contains("ORDER BY"));
        assert!(q.sql.contains("#>>"));
        assert!(q.sql.contains("{user,profile,email}"));
        assert!(q.sql.contains("posts")); // Correct table name
    }

    // ============================================================================
    // Column Validation Tests
    // ============================================================================

    #[test]
    fn test_filter_nonexistent_column_rejected() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            filters: [("nonexistent.field".to_string(), "value".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let result = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Postgres,
            &RequestContext::new(),
        );

        // Verify error is returned for nonexistent column
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn test_sort_nonexistent_column_rejected() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            sort: Some("nonexistent.field".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let result = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Postgres,
            &RequestContext::new(),
        );

        // Verify error is returned for nonexistent sort field
        assert!(result.is_err());
    }

    #[test]
    fn test_filter_nonexistent_jsonb_column_rejected() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let params = QueryParams {
            filters: [("other_column.nested".to_string(), "value".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let result = build_select_list(
            "posts",
            &table,
            &crud,
            &params,
            DatabaseDriver::Postgres,
            &RequestContext::new(),
        );

        // Verify error is returned for nonexistent JSONB column
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }
}

/// Dynamic query builder for CRUD operations driven by config.
///
/// Builds parameterized SQL queries (SELECT, INSERT, UPDATE, DELETE) from
/// endpoint CRUD configuration and request parameters. All user inputs are
/// passed as bind parameters - **never** interpolated into SQL strings.
///
/// SELECT queries (list and single-get) are built through a shared
/// [`SelectBuilder`] pipeline so that clauses like `where_clause`, joins,
/// and computed fields are applied consistently.
use std::collections::HashMap;

use crate::config::types::{ColumnType, CrudConfig, DatabaseDriver, SortOrder, TableConfig};
use crate::context::RequestContext;
use crate::error::AppError;

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
    pub order: Option<SortOrder>,
    /// Filter values: `column_name` -> value.
    pub filters: HashMap<String, String>,
}

/// A built query ready for execution.
#[derive(Debug)]
pub struct BuiltQuery {
    pub sql: String,
    pub params: Vec<serde_json::Value>,
}

// =============================================================================
// SelectBuilder - shared pipeline for all SELECT queries
// =============================================================================

/// Accumulates the clauses of a SELECT statement so that both list and
/// single-get queries share the same logic for fields, joins, computed
/// fields, and WHERE conditions.
struct SelectBuilder {
    table: String,
    select_fields: Vec<String>,
    joins: Vec<String>,
    computed: Vec<String>,
    conditions: Vec<String>,
    order_by: Option<String>,
    limit_offset: Option<(String, String)>,
    params: Vec<serde_json::Value>,
    param_idx: usize,
    driver: DatabaseDriver,
}

impl SelectBuilder {
    /// Start a new SELECT against `table` with the given main-table fields.
    fn new(table: &str, fields: Vec<String>, driver: DatabaseDriver) -> Self {
        Self {
            table: table.to_string(),
            select_fields: fields,
            joins: Vec::new(),
            computed: Vec::new(),
            conditions: Vec::new(),
            order_by: None,
            limit_offset: None,
            params: Vec::new(),
            param_idx: 1,
            driver,
        }
    }

    /// Qualify bare column names with the main table name (needed when joins
    /// introduce columns with the same name).
    fn qualify_main_fields(&mut self) {
        self.select_fields = self
            .select_fields
            .iter()
            .map(|f| {
                if f.contains('.') {
                    f.clone()
                } else {
                    format!("{}.{}", self.table, f)
                }
            })
            .collect();
    }

    /// Append JOIN clauses and their requested fields.
    fn apply_joins(&mut self, crud: &CrudConfig) {
        if crud.joins.is_empty() {
            return;
        }
        self.qualify_main_fields();

        for join in &crud.joins {
            let keyword = match join.join_type {
                crate::config::types::JoinType::Inner => "INNER JOIN",
                crate::config::types::JoinType::Left => "LEFT JOIN",
                crate::config::types::JoinType::Right => "RIGHT JOIN",
            };
            self.joins
                .push(format!("{} {} ON {}", keyword, join.table, join.on));
            for f in &join.fields {
                self.select_fields.push(f.clone());
            }
        }
    }

    /// Append computed (virtual) fields as SQL expressions in the SELECT list.
    fn apply_computed_fields(&mut self, crud: &CrudConfig) {
        for cf in &crud.computed_fields {
            self.computed
                .push(format!("{} AS {}", cf.expression, cf.name));
        }
    }

    /// Append the `where_clause` from config, resolving dynamic parameters if present.
    fn apply_where_clause(
        &mut self,
        crud: &CrudConfig,
        context: &RequestContext,
    ) -> Result<(), AppError> {
        if let Some(ref wc) = crud.where_clause {
            let interpolated = self.interpolate_where_clause(wc, context)?;
            self.conditions.push(format!("({interpolated})"));
        }
        Ok(())
    }

    /// Interpolate ${key} patterns in a string using the provided `RequestContext`.
    fn interpolate_where_clause(
        &mut self,
        wc: &str,
        context: &RequestContext,
    ) -> Result<String, AppError> {
        use regex::Regex;
        let re = Regex::new(r"\$\{([^}]+)\}")
            .map_err(|e| AppError::Internal(format!("Invalid interpolation regex: {e}")))?;
        let mut last_match_end = 0;
        let mut new_string = String::new();

        for cap in re.captures_iter(wc) {
            let full_match = cap
                .get(0)
                .ok_or_else(|| AppError::Internal("Regex match failed".to_string()))?;
            let key = cap
                .get(1)
                .ok_or_else(|| AppError::Internal("Regex capture failed".to_string()))?
                .as_str();

            new_string.push_str(&wc[last_match_end..full_match.start()]);

            let mut resolved_value = self.resolve_context_key(key, context);

            // Handle default values: ${key:-default}
            if resolved_value.is_none()
                && key.contains(":-")
                && let Some(idx) = key.find(":-")
            {
                let base_key = &key[..idx];
                let default_val = &key[idx + 2..];
                resolved_value = self
                    .resolve_context_key(base_key, context)
                    .or(Some(default_val.to_string()));
            }

            if let Some(val) = resolved_value {
                let ph = crate::db::query::placeholder(self.driver, self.param_idx);
                self.params.push(serde_json::Value::String(val));
                self.param_idx += 1;
                new_string.push_str(&ph);
            } else {
                return Err(AppError::BadRequest(format!(
                    "Could not resolve interpolation key: {key}"
                )));
            }

            last_match_end = full_match.end();
        }

        new_string.push_str(&wc[last_match_end..]);
        Ok(new_string)
    }

    /// Helper to resolve a single context key.
    fn resolve_context_key(&self, key: &str, context: &RequestContext) -> Option<String> {
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

    /// Append user-supplied filter conditions as parameterized WHERE terms.
    fn apply_filters(
        &mut self,
        crud: &CrudConfig,
        filters: &HashMap<String, String>,
        _context: &RequestContext,
    ) -> Result<(), AppError> {
        if !crud.filtering.enabled {
            return Ok(());
        }
        for (key, value) in filters {
            if !is_valid_identifier(key) {
                return Err(AppError::BadRequest(format!("Invalid filter field: {key}")));
            }
            let allowed = &crud.filtering.allowed_fields;
            if !allowed.contains(&"*".to_string()) && !allowed.contains(key) {
                return Err(AppError::BadRequest(format!(
                    "Filtering on '{key}' is not allowed"
                )));
            }
            self.conditions.push(format!(
                "{} = {}",
                key,
                placeholder(self.driver, self.param_idx)
            ));
            self.params.push(serde_json::Value::String(value.clone()));
            self.param_idx += 1;
        }
        Ok(())
    }

    /// Add a single PK equality condition.
    fn apply_pk_condition(&mut self, pk_col: &str, pk_value: serde_json::Value) {
        self.conditions.push(format!(
            "{} = {}",
            pk_col,
            placeholder(self.driver, self.param_idx)
        ));
        self.params.push(pk_value);
        self.param_idx += 1;
    }

    /// Set the ORDER BY clause from config + request params.
    fn apply_sorting(
        &mut self,
        crud: &CrudConfig,
        table_config: &TableConfig,
        query_params: &QueryParams,
    ) -> Result<(), AppError> {
        if !crud.sorting.enabled {
            return Ok(());
        }
        let sort_field = query_params.sort.as_deref().unwrap_or_else(|| {
            if crud.sorting.default_field.is_empty() {
                table_config
                    .columns
                    .iter()
                    .find(|c| c.primary_key)
                    .map_or("id", |c| c.name.as_str())
            } else {
                &crud.sorting.default_field
            }
        });

        if !is_valid_identifier(sort_field) {
            return Err(AppError::BadRequest(format!(
                "Invalid sort field: {sort_field}"
            )));
        }
        let allowed = &crud.sorting.allowed_fields;
        if !allowed.contains(&"*".to_string()) && !allowed.contains(&sort_field.to_string()) {
            return Err(AppError::BadRequest(format!(
                "Sorting by '{sort_field}' is not allowed"
            )));
        }

        let order = query_params.order.unwrap_or(crud.sorting.default_order);
        let order_str = match order {
            SortOrder::Asc => "ASC",
            SortOrder::Desc => "DESC",
        };
        let qualified = if !self.joins.is_empty() && !sort_field.contains('.') {
            format!("{}.{}", self.table, sort_field)
        } else {
            sort_field.to_string()
        };
        self.order_by = Some(format!("{qualified} {order_str}"));
        Ok(())
    }

    /// Set LIMIT/OFFSET from pagination config + request params.
    fn apply_pagination(&mut self, crud: &CrudConfig, query_params: &QueryParams) {
        if !crud.pagination.enabled {
            return;
        }
        let page_size = query_params
            .page_size
            .unwrap_or(crud.pagination.default_page_size)
            .min(crud.pagination.max_page_size);
        let page = query_params.page.unwrap_or(1).max(1);
        let offset = (page - 1) * page_size;

        let limit_ph = placeholder(self.driver, self.param_idx);
        let offset_ph = placeholder(self.driver, self.param_idx + 1);
        self.param_idx += 2;
        self.params.push(serde_json::json!(
            i64::try_from(page_size).unwrap_or(i64::MAX)
        ));
        self.params
            .push(serde_json::json!(i64::try_from(offset).unwrap_or(i64::MAX)));
        self.limit_offset = Some((limit_ph, offset_ph));
    }

    /// Set a hard LIMIT 1 (for single-record lookups).
    fn limit_one(&mut self) {
        // Use a literal "1" rather than a param - no user input involved.
        self.limit_offset = Some(("1".to_string(), "0".to_string()));
    }

    /// Render the final SQL string and return params.
    fn build(self) -> BuiltQuery {
        let mut select = self.select_fields;
        for c in &self.computed {
            select.push(c.clone());
        }

        let mut sql = format!("SELECT {} FROM {}", select.join(", "), self.table);

        for j in &self.joins {
            sql.push_str(&format!(" {j}"));
        }

        if !self.conditions.is_empty() {
            sql.push_str(&format!(" WHERE {}", self.conditions.join(" AND ")));
        }

        if let Some(ref ob) = self.order_by {
            sql.push_str(&format!(" ORDER BY {ob}"));
        }

        if let Some((ref limit, ref offset)) = self.limit_offset {
            if offset == "0" && !limit.starts_with('$') && !limit.starts_with('?') {
                // Literal LIMIT (e.g. "LIMIT 1") - skip OFFSET.
                sql.push_str(&format!(" LIMIT {limit}"));
            } else {
                sql.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));
            }
        }

        BuiltQuery {
            sql,
            params: self.params,
        }
    }
}

// =============================================================================
// Public query builders
// =============================================================================

/// Build a SELECT query for listing records.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if filter or sort fields are invalid or disallowed.
pub fn build_select_list(
    table_name: &str,
    table_config: &TableConfig,
    crud: &CrudConfig,
    query_params: &QueryParams,
    driver: DatabaseDriver,
    context: &RequestContext,
) -> Result<BuiltQuery, AppError> {
    let fields = resolve_fields(&crud.fields, table_config);
    let mut sb = SelectBuilder::new(table_name, fields, driver);

    sb.apply_joins(crud);
    sb.apply_computed_fields(crud);
    sb.apply_where_clause(crud, context)?;
    sb.apply_filters(crud, &query_params.filters, context)?;
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
    let fields = resolve_fields(&crud.fields, table_config);
    let pk_col = find_pk_column(table_config)?;
    let mut sb = SelectBuilder::new(table_name, fields, driver);

    sb.apply_pk_condition(&pk_col, coerce_pk_value(table_config, pk_value));
    sb.apply_where_clause(crud, context)?;
    sb.limit_one();

    Ok(sb.build())
}

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

// =============================================================================
// Helpers
// =============================================================================

/// Helper to interpolate a string value.
/// Helper to interpolate a string value.
fn interpolate_value(value: &str, context: &RequestContext) -> Result<serde_json::Value, AppError> {
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

/// Helper to resolve a single context key (shared logic with `SelectBuilder`).
fn resolve_single_key(key: &str, context: &RequestContext) -> Option<String> {
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
fn placeholder(driver: DatabaseDriver, index: usize) -> String {
    match driver {
        DatabaseDriver::Postgres => format!("${index}"),
        DatabaseDriver::Sqlite | DatabaseDriver::Mysql => "?".to_string(),
    }
}

/// Resolve field list: ["*"] -> all column names, otherwise as-is.
fn resolve_fields(fields: &[String], table: &TableConfig) -> Vec<String> {
    if fields.len() == 1 && fields[0] == "*" {
        table.columns.iter().map(|c| c.name.clone()).collect()
    } else {
        fields.to_vec()
    }
}

/// Resolve writable fields: empty list -> all non-PK columns.
fn resolve_writable_fields(writable: &[String], table: &TableConfig) -> Vec<String> {
    if writable.is_empty() {
        table
            .columns
            .iter()
            .filter(|c| !c.primary_key)
            .map(|c| c.name.clone())
            .collect()
    } else {
        writable.to_vec()
    }
}

/// Find the primary key column name.
fn find_pk_column(table: &TableConfig) -> Result<String, AppError> {
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
fn coerce_pk_value(table: &TableConfig, raw: &str) -> serde_json::Value {
    let col_type = table
        .columns
        .iter()
        .find(|c| c.primary_key)
        .map(|c| c.column_type);

    match col_type {
        Some(
            ColumnType::Integer
            | ColumnType::Bigint
            | ColumnType::Smallint
            | ColumnType::Serial
            | ColumnType::Bigserial,
        ) => raw.parse::<i64>().map_or_else(
            |_| serde_json::Value::String(raw.to_string()),
            |n| serde_json::json!(n),
        ),
        Some(ColumnType::Float | ColumnType::Double | ColumnType::Decimal) => {
            raw.parse::<f64>().map_or_else(
                |_| serde_json::Value::String(raw.to_string()),
                |n| serde_json::json!(n),
            )
        }
        _ => serde_json::Value::String(raw.to_string()),
    }
}

/// Validate that a string is a safe SQL identifier (prevents injection).
/// Allows alphanumeric, underscores, dots (for table.column), and hyphens.
fn is_valid_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::*;

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
        assert_eq!(
            q.sql,
            "SELECT id, title, author FROM posts ORDER BY id ASC LIMIT ? OFFSET ?"
        );
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
        assert_eq!(
            q.sql,
            "SELECT id, title, author FROM posts WHERE author = ? ORDER BY id ASC LIMIT ? OFFSET ?"
        );
        assert_eq!(
            q.params,
            vec![
                serde_json::json!("alice"),
                serde_json::json!(20i64),
                serde_json::json!(0i64)
            ]
        );
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
        assert_eq!(
            q.sql,
            "SELECT id, title, author FROM posts WHERE author = $1 ORDER BY id ASC LIMIT $2 OFFSET $3"
        );
        assert_eq!(
            q.params,
            vec![
                serde_json::json!("bob"),
                serde_json::json!(20i64),
                serde_json::json!(0i64)
            ]
        );
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
        // BTreeMap iteration is alphabetical: author before title.
        assert_eq!(q.sql, "INSERT INTO posts (author, title) VALUES (?, ?)");
        assert_eq!(
            q.params,
            vec![serde_json::json!("Alice"), serde_json::json!("Hello")]
        );
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
        // "id" should be excluded since it's not in writable_fields.
        assert_eq!(q.sql, "INSERT INTO posts (author, title) VALUES (?, ?)");
        assert_eq!(
            q.params,
            vec![serde_json::json!("Alice"), serde_json::json!("Hello")]
        );
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
        assert_eq!(
            q.sql,
            "INSERT INTO posts (author, title) VALUES ($1, $2) RETURNING id"
        );
        assert_eq!(
            q.params,
            vec![serde_json::json!("Alice"), serde_json::json!("Hello")]
        );
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
        let resolved = resolve_writable_fields(&[], &table);
        assert_eq!(resolved, vec!["title", "author"]);
        assert!(!resolved.contains(&"id".to_string()));
    }
}

/// Core SELECT query builder pipeline.
///
/// Accumulates the clauses of a SELECT statement so that both list and
/// single-get queries share the same logic for fields, joins, computed
/// fields, and WHERE conditions.
use std::collections::HashMap;

use crate::config::types::{ColumnType, CrudConfig, DatabaseDriver, SortOrder, TableConfig};
use crate::context::RequestContext;
use crate::db::query::helpers::{
    FilterExpression, FilterOperator, build_filter_param, extract_base_column,
    extract_jsonb_sort_path, is_bracket_notation, is_jsonb_column, is_jsonb_path,
    is_valid_expression, is_valid_filter_column, is_valid_sort_field, parse_filter_key,
    parse_sort_field, placeholder,
};
use crate::db::query::traits::{FilterBehavior, MysqlFilter, PostgresFilter, SqliteFilter};
use crate::db::query::types::{BuiltQuery, QueryParams};
use crate::error::AppError;

impl SelectBuilder {
    /// Get the filter behavior for this driver.
    fn filter_behavior(&self) -> Box<dyn FilterBehavior> {
        match self.driver {
            DatabaseDriver::Postgres => Box::new(PostgresFilter),
            DatabaseDriver::Mysql => Box::new(MysqlFilter),
            DatabaseDriver::Sqlite => Box::new(SqliteFilter),
        }
    }
}

pub struct SelectBuilder {
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
    pub fn new(table: &str, fields: Vec<String>, driver: DatabaseDriver) -> Self {
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
    pub fn qualify_main_fields(&mut self) {
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
    pub fn apply_joins(&mut self, crud: &CrudConfig) {
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
                // Preserve fully qualified field names (e.g., "authors.name")
                // or qualify with the main table name for bare column names
                if f.contains('.') {
                    self.select_fields.push(f.clone());
                } else {
                    self.select_fields.push(format!("{}.{}", self.table, f));
                }
            }
        }
    }

    /// Append computed (virtual) fields as SQL expressions in the SELECT list.
    pub fn apply_computed_fields(&mut self, crud: &CrudConfig) {
        for cf in &crud.computed_fields {
            self.computed
                .push(format!("{} AS {}", cf.expression, cf.name));
        }
    }

    /// Append the `where_clause` from config, resolving dynamic parameters if present.
    pub fn apply_where_clause(
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
                let ph = placeholder(self.driver, self.param_idx);
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
    pub fn apply_filters(
        &mut self,
        crud: &CrudConfig,
        filters: &HashMap<String, String>,
        table_config: &TableConfig,
        _context: &RequestContext,
    ) -> Result<(), AppError> {
        for (key, value) in filters {
            let expr = parse_filter_key(key)?;

            let base_column = expr
                .path
                .first()
                .ok_or_else(|| AppError::BadRequest(format!("Invalid filter key: {key}")))?;

            let allowed = &crud.filtering.allowed_fields;
            // For allowed_fields check, extract the base column name
            let allowed_base = extract_base_column(base_column);
            if !allowed.contains(&"*".to_string()) && !allowed.contains(&allowed_base) {
                return Err(AppError::BadRequest(format!(
                    "Filtering by '{key}' is not allowed"
                )));
            }

            // Validate that the filter column exists in the table schema
            if !is_valid_filter_column(key, &table_config.columns) {
                return Err(AppError::BadRequest(format!(
                    "Filter column '{key}' does not exist in table '{}'",
                    self.table
                )));
            }

            // Get the column type for proper value coercion
            let column_type = self.get_column_type_for_filter(key, &table_config.columns);
            self.apply_filter_expression(expr, value, column_type)?;
        }
        Ok(())
    }

    /// Get the column type for a filter key.
    ///
    /// For JSONB/JSON columns with nested paths (e.g., `metadata.role`),
    /// this returns the base column's type (Jsonb or Json).
    /// For regular columns, it returns the column's declared type.
    fn get_column_type_for_filter<'a>(
        &self,
        filter_key: &str,
        columns: &'a [crate::config::types::ColumnConfig],
    ) -> Option<&'a crate::config::types::ColumnType> {
        let base = extract_base_column(filter_key);
        columns
            .iter()
            .find(|c| c.name == base)
            .map(|c| &c.column_type)
    }

    /// Applies a single filter expression to the query.
    fn apply_filter_expression(
        &mut self,
        expr: FilterExpression,
        value: &str,
        column_type: Option<&crate::config::types::ColumnType>,
    ) -> Result<(), AppError> {
        let behavior = self.filter_behavior();
        self.apply_filter_common(expr, value, column_type, &*behavior)
    }

    /// Unified filter logic using the FilterBehavior trait.
    fn apply_filter_common(
        &mut self,
        expr: FilterExpression,
        value: &str,
        column_type: Option<&crate::config::types::ColumnType>,
        behavior: &dyn FilterBehavior,
    ) -> Result<(), AppError> {
        let path_str = expr.path.join(".");
        let base_column = expr.path.first().cloned().unwrap_or_default();
        let is_jsonb_field = expr.path.len() > 1
            || column_type.is_some_and(|ct| matches!(ct, ColumnType::Jsonb | ColumnType::Json));

        let values: Vec<String> = match expr.operator {
            FilterOperator::In | FilterOperator::NotIn => {
                value.split(',').map(|s| s.trim().to_string()).collect()
            }
            FilterOperator::Contains => {
                if is_jsonb_field {
                    vec![value.to_string()]
                } else {
                    vec![format!("%{}%", value)]
                }
            }
            FilterOperator::StartsWith => {
                vec![format!("{}%", value)]
            }
            FilterOperator::EndsWith => {
                vec![format!("%{}", value)]
            }
            _ => vec![value.to_string()],
        };
        let num_params = values.len();

        let condition = match expr.operator {
            FilterOperator::Eq => self.build_comparison(
                &base_column,
                is_jsonb_field,
                &placeholder(self.driver, self.param_idx),
                behavior.eq_op(),
                &path_str,
                behavior,
            ),
            FilterOperator::Ne => self.build_comparison(
                &base_column,
                is_jsonb_field,
                &placeholder(self.driver, self.param_idx),
                behavior.ne_op(),
                &path_str,
                behavior,
            ),
            FilterOperator::Gt => self.build_comparison(
                &base_column,
                is_jsonb_field,
                &placeholder(self.driver, self.param_idx),
                behavior.gt_op(),
                &path_str,
                behavior,
            ),
            FilterOperator::Gte => self.build_comparison(
                &base_column,
                is_jsonb_field,
                &placeholder(self.driver, self.param_idx),
                behavior.gte_op(),
                &path_str,
                behavior,
            ),
            FilterOperator::Lt => self.build_comparison(
                &base_column,
                is_jsonb_field,
                &placeholder(self.driver, self.param_idx),
                behavior.lt_op(),
                &path_str,
                behavior,
            ),
            FilterOperator::Lte => self.build_comparison(
                &base_column,
                is_jsonb_field,
                &placeholder(self.driver, self.param_idx),
                behavior.lte_op(),
                &path_str,
                behavior,
            ),
            FilterOperator::In => {
                let placeholders: Vec<String> = (0..num_params)
                    .map(|i| placeholder(self.driver, self.param_idx + i))
                    .collect();
                let param_list = placeholders.join(", ");
                self.build_in(
                    &base_column,
                    is_jsonb_field,
                    &param_list,
                    &path_str,
                    behavior,
                )
            }
            FilterOperator::NotIn => {
                let placeholders: Vec<String> = (0..num_params)
                    .map(|i| placeholder(self.driver, self.param_idx + i))
                    .collect();
                let param_list = placeholders.join(", ");
                let condition = self.build_in(
                    &base_column,
                    is_jsonb_field,
                    &param_list,
                    &path_str,
                    behavior,
                );
                format!("NOT {condition}")
            }
            FilterOperator::Contains => self.build_contains(
                is_jsonb_field,
                &placeholder(self.driver, self.param_idx),
                &path_str,
                value,
                behavior,
            )?,
            FilterOperator::Exists => {
                self.build_exists(is_jsonb_field, &base_column, &path_str, behavior)?
            }
            FilterOperator::StartsWith => self.build_like(
                &base_column,
                is_jsonb_field,
                &placeholder(self.driver, self.param_idx),
                &path_str,
                behavior,
            ),
            FilterOperator::EndsWith => self.build_like(
                &base_column,
                is_jsonb_field,
                &placeholder(self.driver, self.param_idx),
                &path_str,
                behavior,
            ),
            FilterOperator::Like => self.build_like(
                &base_column,
                is_jsonb_field,
                &placeholder(self.driver, self.param_idx),
                &path_str,
                behavior,
            ),
            FilterOperator::ILike => self.build_ilike(
                &base_column,
                is_jsonb_field,
                &placeholder(self.driver, self.param_idx),
                &path_str,
                behavior,
            ),
        };

        self.conditions.push(condition);

        for v in &values {
            let param_value = build_filter_param(v, column_type);
            self.params.push(param_value);
        }
        self.param_idx += num_params;

        Ok(())
    }

    /// Build a simple comparison condition (e.g., column = param).
    fn build_comparison(
        &self,
        base_column: &str,
        is_jsonb: bool,
        param: &str,
        operator: &str,
        path_str: &str,
        behavior: &dyn FilterBehavior,
    ) -> String {
        if is_jsonb {
            // For JSONB, extract the column name and the nested path
            // path_str is "metadata.role", we need column="metadata", nested_path="role"
            let parts: Vec<&str> = path_str.split('.').collect();
            let column_name = parts.first().unwrap_or(&base_column);
            let nested_path = if parts.len() > 1 {
                parts[1..].join(".")
            } else {
                String::new()
            };
            format!(
                "{} {} {}",
                behavior.json_extract_path(column_name, &nested_path),
                operator,
                param
            )
        } else {
            format!("{}.{} {} {}", self.table, base_column, operator, param)
        }
    }

    /// Build an IN/NOT IN condition.
    fn build_in(
        &self,
        base_column: &str,
        is_jsonb: bool,
        param_list: &str,
        path_str: &str,
        behavior: &dyn FilterBehavior,
    ) -> String {
        if is_jsonb {
            // Extract the column name from the path (e.g., "metadata" from "metadata.role")
            let column_name = path_str.split('.').next().unwrap_or(base_column);
            format!(
                "{} IN ({})",
                behavior.json_extract_path(column_name, path_str),
                param_list
            )
        } else {
            format!("{}.{} IN ({})", self.table, base_column, param_list)
        }
    }

    /// Build a CONTAINS condition.
    fn build_contains(
        &self,
        is_jsonb: bool,
        param: &str,
        path_str: &str,
        value: &str,
        behavior: &dyn FilterBehavior,
    ) -> Result<String, AppError> {
        let column_name = path_str.split('.').next().ok_or_else(|| {
            AppError::Internal("Invalid JSONB path: empty path string".to_string())
        })?;
        if is_jsonb {
            let json_value = serde_json::Value::String(value.to_string());
            let json_str = serde_json::to_string(&json_value)
                .map_err(|e| AppError::Internal(format!("JSON serialization error: {e}")))?;
            if behavior.uses_jsonb_ops() {
                // PostgreSQL @> operator for JSONB containment
                Ok(format!("{} @> '{}'::jsonb", column_name, json_str))
            } else {
                Ok(format!(
                    "EXISTS (SELECT 1 FROM json_each({}, '$.{}') WHERE value = {})",
                    column_name, path_str, param
                ))
            }
        } else {
            Ok(format!("{}.{} LIKE {}", self.table, column_name, param))
        }
    }

    /// Build an EXISTS condition.
    fn build_exists(
        &self,
        is_jsonb: bool,
        base_column: &str,
        path_str: &str,
        behavior: &dyn FilterBehavior,
    ) -> Result<String, AppError> {
        if is_jsonb {
            // Extract the column name from path_str (e.g., "metadata" from "metadata.role")
            let column_name = path_str.split('.').next().ok_or_else(|| {
                AppError::Internal("Invalid JSONB path: empty path string".to_string())
            })?;
            Ok(format!(
                "{} IS NOT NULL",
                behavior.json_extract_path(column_name, path_str)
            ))
        } else {
            Ok(format!("{}.{} IS NOT NULL", self.table, base_column))
        }
    }

    /// Build a LIKE condition.
    fn build_like(
        &self,
        base_column: &str,
        is_jsonb: bool,
        param: &str,
        path_str: &str,
        behavior: &dyn FilterBehavior,
    ) -> String {
        if is_jsonb {
            // Extract the column name from the path (e.g., "metadata" from "metadata.role")
            let column_name = path_str.split('.').next().unwrap_or(base_column);
            format!(
                "{} {} {}",
                behavior.json_extract_path(column_name, path_str),
                behavior.like_op(),
                param
            )
        } else {
            format!(
                "{}.{} {} {}",
                self.table,
                base_column,
                behavior.like_op(),
                param
            )
        }
    }

    /// Build an ILIKE (case-insensitive LIKE) condition.
    fn build_ilike(
        &self,
        base_column: &str,
        is_jsonb: bool,
        value: &str,
        path_str: &str,
        behavior: &dyn FilterBehavior,
    ) -> String {
        let param = format!("%{}%", value);
        if is_jsonb {
            // Extract the column name from the path (e.g., "metadata" from "metadata.role")
            let column_name = path_str.split('.').next().unwrap_or(base_column);
            format!(
                "LOWER({}) {} LOWER({})",
                behavior.json_extract_path(column_name, path_str),
                behavior.ilike_op(),
                param
            )
        } else {
            format!(
                "LOWER({}.{}) {} LOWER({})",
                self.table,
                base_column,
                behavior.ilike_op(),
                param
            )
        }
    }

    /// Add a single PK equality condition.
    ///
    /// If the pk_value is the sentinel `i64::MIN` (indicating coercion failed
    /// for an integer PK), uses `1 = 0` to ensure no rows match, avoiding
    /// type mismatch errors on PostgreSQL when binding a non-numeric string
    /// to an integer column.
    pub fn apply_pk_condition(&mut self, pk_col: &str, pk_value: serde_json::Value) {
        let is_coercion_sentinel = pk_value
            .as_number()
            .is_some_and(|n| n.as_i64() == Some(i64::MIN));
        if is_coercion_sentinel {
            self.conditions.push("1 = 0".to_string());
        } else {
            self.conditions.push(format!(
                "{} = {}",
                pk_col,
                placeholder(self.driver, self.param_idx)
            ));
            self.params.push(pk_value);
            self.param_idx += 1;
        }
    }

    /// Set the ORDER BY clause from config + request params.
    pub fn apply_sorting(
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

        if !is_valid_expression(sort_field) {
            return Err(AppError::BadRequest(format!(
                "Invalid sort field: {sort_field}"
            )));
        }

        if !is_valid_sort_field(sort_field, &table_config.columns) {
            return Err(AppError::BadRequest(format!(
                "Sort field '{sort_field}' does not exist in table '{}'",
                self.table
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

        let (base_col, _path) = parse_sort_field(sort_field);

        let order_clause = if is_jsonb_path(sort_field) || is_bracket_notation(sort_field) {
            if is_jsonb_column(&base_col, &table_config.columns) {
                let path_str = extract_jsonb_sort_path(sort_field);
                let jsonb_expr = match self.driver {
                    DatabaseDriver::Postgres => {
                        // PostgreSQL #>> operator expects text array syntax {a,b}, not JSONPath $.a.b
                        // Convert $.role to {role} or $.user.profile to {user,profile}
                        let pg_path = path_str.strip_prefix("$.").unwrap_or(&path_str);
                        let array_syntax: String =
                            pg_path.split('.').collect::<Vec<&str>>().join(",");
                        format!("({} #>> '{{{}}}')", base_col, array_syntax)
                    }
                    DatabaseDriver::Mysql => {
                        format!("JSON_EXTRACT({}, '{}')", base_col, path_str)
                    }
                    DatabaseDriver::Sqlite => {
                        format!("json_extract({}, '{}')", base_col, path_str)
                    }
                };
                format!("{jsonb_expr} {order_str}")
            } else {
                let qualified = if !self.joins.is_empty() && !sort_field.contains('.') {
                    format!("{}.{}", self.table, sort_field)
                } else {
                    sort_field.to_string()
                };
                format!("{qualified} {order_str}")
            }
        } else {
            let qualified = if !self.joins.is_empty() && !sort_field.contains('.') {
                format!("{}.{}", self.table, sort_field)
            } else {
                sort_field.to_string()
            };
            format!("{qualified} {order_str}")
        };

        self.order_by = Some(order_clause);
        Ok(())
    }

    /// Set LIMIT/OFFSET from pagination config + request params.
    pub fn apply_pagination(&mut self, crud: &CrudConfig, query_params: &QueryParams) {
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
    pub fn limit_one(&mut self) {
        // Use a literal "1" rather than a param - no user input involved.
        self.limit_offset = Some(("1".to_string(), "0".to_string()));
    }

    /// Render the final SQL string and return params.
    pub fn build(self) -> BuiltQuery {
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

use crate::config::types::{ColumnType, CrudConfig, DatabaseDriver};
use crate::db::query::helpers::{
    coerce_filter_value_by_type, coerce_pk_value, find_pk_column, interpolate_value,
    is_valid_identifier, placeholder, quote_identifier, resolve_writable_fields,
};
use crate::db::query::select::SelectBuilder;
use crate::db::query::types::{BuiltQuery, MutationContext, SelectContext};
use crate::error::AppError;
use crate::handlers::common::utils::DatabaseContext;
use crate::middleware::auth::extractor::RequestContext;

impl From<&CrudConfig> for MutationContext {
    /// Constructs a mutation context from the CRUD configuration's writable fields and where clauses.
    fn from(crud: &CrudConfig) -> Self {
        MutationContext {
            writable_fields: crud.writable_fields.clone(),
            insert_owner: crud.insert_owner.clone(),
            update_where_clause: crud.update_where_clause.clone(),
            delete_where_clause: crud.delete_where_clause.clone(),
        }
    }
}

impl From<&CrudConfig> for SelectContext {
    /// Constructs a select context from the CRUD configuration's field and filter settings.
    fn from(crud: &CrudConfig) -> Self {
        SelectContext {
            fields: crud.fields.clone(),
            joins: crud.joins.clone(),
            computed_fields: crud.computed_fields.clone(),
            where_clause: crud.where_clause.clone(),
            filtering_enabled: crud.filtering.enabled,
            filtering_allowed_fields: crud.filtering.allowed_fields.clone(),
            sorting_enabled: crud.sorting.enabled,
            sorting_default_field: crud.sorting.default_field.clone(),
            sorting_default_order: crud.sorting.default_order,
            sorting_allowed_fields: crud.sorting.allowed_fields.clone(),
            pagination_enabled: crud.pagination.enabled,
            pagination_default_page_size: crud.pagination.default_page_size,
            pagination_max_page_size: crud.pagination.max_page_size,
        }
    }
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
    db_ctx: &DatabaseContext,
    ctx: &MutationContext,
    body: &serde_json::Value,
    context: &RequestContext,
) -> Result<BuiltQuery, AppError> {
    let table_config = &db_ctx.table_config;
    let driver = db_ctx.pool.driver();
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::BadRequest("Request body must be a JSON object".to_string()))?;

    let writable = resolve_writable_fields(&ctx.writable_fields, &db_ctx.table_config);

    // Check if we need to auto-populate the owner field.
    let owner_col = ctx.insert_owner.clone();

    let mut columns: Vec<String> = Vec::new();
    let mut placeholders: Vec<String> = Vec::new();
    let mut params: Vec<serde_json::Value> = Vec::new();
    let mut param_idx = 1usize;

    // Auto-populate owner field if configured. Server always controls ownership.
    if let Some(ref owner_field) = owner_col
        && let Some(user_id) = &context.user_id
    {
        let owner_col_type = table_config
            .columns
            .iter()
            .find(|c| c.name == *owner_field)
            .map(|c| &c.column_type);

        let coerced = match owner_col_type {
            Some(ct) => coerce_filter_value_by_type(user_id, ct, db_ctx.pool.driver()),
            None => serde_json::Value::String(user_id.to_string()),
        };
        columns.push(owner_field.clone());
        let placeholder = placeholder(driver, param_idx);
        placeholders.push(placeholder);
        params.push(coerced);
        param_idx += 1;
    }

    for (key, value) in obj {
        // Skip the owner field - it is auto-populated by insert_owner.
        if let Some(ref owner_field) = owner_col
            && key == owner_field
        {
            continue;
        }

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

        let placeholder = if driver == DatabaseDriver::Postgres {
            // Check if this column is a JSONB type
            let is_jsonb = table_config
                .columns
                .iter()
                .any(|c| c.name == *key && matches!(c.column_type, ColumnType::Jsonb));
            if is_jsonb {
                format!("{}::jsonb", placeholder(driver, param_idx))
            } else {
                placeholder(driver, param_idx)
            }
        } else {
            placeholder(driver, param_idx)
        };

        placeholders.push(placeholder);
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
        DatabaseDriver::Postgres | DatabaseDriver::Sqlite => {
            if let Some(ref owner_field) = owner_col {
                format!(
                    " RETURNING {}, {}",
                    quote_identifier(&pk_col, driver),
                    quote_identifier(owner_field, driver)
                )
            } else {
                format!(" RETURNING {}", quote_identifier(&pk_col, driver))
            }
        }
        _ => String::new(),
    };

    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({}){}",
        table_config.name,
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
    db_ctx: &DatabaseContext,
    ctx: MutationContext,
    pk_value: &str,
    body: &serde_json::Value,
    context: &RequestContext,
    where_clause: &Option<String>,
) -> Result<BuiltQuery, AppError> {
    let table_name = &db_ctx.table_config.name;
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::BadRequest("Request body must be a JSON object".to_string()))?;

    let writable = resolve_writable_fields(&ctx.writable_fields, &db_ctx.table_config);
    let pk_col = find_pk_column(&db_ctx.table_config)?;
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

        // Postgres requires an explicit cast to JSONB when binding a text
        // parameter to a JSONB column.
        let set_value = if db_ctx.pool.driver() == DatabaseDriver::Postgres
            && db_ctx
                .table_config
                .columns
                .iter()
                .any(|c| c.name == *key && matches!(c.column_type, ColumnType::Jsonb))
        {
            format!("{}::jsonb", placeholder(db_ctx.pool.driver(), param_idx))
        } else {
            placeholder(db_ctx.pool.driver(), param_idx)
        };

        set_parts.push(format!("{} = {}", key, set_value));
        params.push(final_value);
        param_idx += 1;
    }

    if set_parts.is_empty() {
        return Err(AppError::BadRequest(
            "No writable fields provided in request body".to_string(),
        ));
    }

    let pk_val = coerce_pk_value(&db_ctx.table_config, pk_value);
    let is_coercion_sentinel = pk_val
        .as_number()
        .is_some_and(|n| n.as_i64() == Some(i64::MIN));
    // "1 = 0" is a sentinel WHERE clause that matches zero rows.
    // This is used when PK coercion fails (e.g. type mismatch) to prevent
    // the UPDATE from affecting any row instead of failing silently.
    let mut sql = if is_coercion_sentinel {
        format!(
            "UPDATE {} SET {} WHERE 1 = 0",
            quote_identifier(table_name, db_ctx.pool.driver()),
            set_parts.join(", ")
        )
    } else {
        format!(
            "UPDATE {} SET {} WHERE {} = {}",
            table_name,
            set_parts.join(", "),
            pk_col,
            placeholder(db_ctx.pool.driver(), param_idx)
        )
    };
    if !is_coercion_sentinel {
        params.push(pk_val);
    }

    if let Some(wc) = where_clause {
        let interpolated = interpolate_value(wc, context)?;
        let wc_str = match interpolated {
            serde_json::Value::String(s) => s,
            other => other.to_string(),
        };
        sql = format!("{sql} AND {wc_str}");
    }

    Ok(BuiltQuery { sql, params })
}

/// Build a DELETE query targeting a single record by PK.
///
/// # Errors
///
/// Returns `AppError::Internal` if the table has no primary key column.
pub fn build_delete(
    pk_value: &str,
    db_ctx: &DatabaseContext,
    context: &RequestContext,
    where_clause: &Option<String>,
) -> Result<BuiltQuery, AppError> {
    let table_name = &db_ctx.table_config.name;
    let pk_col = find_pk_column(&db_ctx.table_config)?;
    let pk_val = coerce_pk_value(&db_ctx.table_config, pk_value);
    let is_coercion_sentinel = pk_val
        .as_number()
        .is_some_and(|n| n.as_i64() == Some(i64::MIN));

    if is_coercion_sentinel {
        let sql = if let Some(wc) = where_clause {
            let interpolated = interpolate_value(wc, context)?;
            let wc_str = match interpolated {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            };
            format!(
                "DELETE FROM {} WHERE 1 = 0 AND {wc_str}",
                quote_identifier(table_name, db_ctx.pool.driver())
            )
        } else {
            format!(
                "DELETE FROM {} WHERE 1 = 0",
                quote_identifier(table_name, db_ctx.pool.driver())
            )
        };
        return Ok(BuiltQuery {
            sql,
            params: Vec::new(),
        });
    }

    let params: Vec<serde_json::Value> = vec![pk_val];
    let mut base_sql = format!(
        "DELETE FROM {} WHERE {} = {}",
        table_name,
        pk_col,
        placeholder(db_ctx.pool.driver(), 1)
    );

    if let Some(wc) = where_clause {
        let interpolated = interpolate_value(wc, context)?;
        let wc_str = match interpolated {
            serde_json::Value::String(s) => s,
            other => other.to_string(),
        };
        base_sql = format!("{base_sql} AND {wc_str}");
    }

    Ok(BuiltQuery {
        sql: base_sql,
        params,
    })
}

/// Build a SELECT query for listing records.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if filter or sort fields are invalid or disallowed.
pub fn build_select_list(
    db_ctx: &DatabaseContext,
    ctx: &SelectContext,
    query_params: &crate::db::query::types::QueryParams,
    context: &RequestContext,
) -> Result<BuiltQuery, AppError> {
    let table_name = &db_ctx.table_config.name;
    use crate::db::query::helpers::resolve_fields;

    let fields = resolve_fields(&ctx.fields, &db_ctx.table_config);
    let mut sb = SelectBuilder::new(table_name, fields, db_ctx.pool.driver());

    sb.apply_joins(ctx);
    sb.apply_computed_fields(ctx);
    sb.apply_where_clause(ctx, context)?;
    sb.apply_filters(ctx, &query_params.filters, &db_ctx.table_config)?;
    sb.apply_sorting(ctx, &db_ctx.table_config, query_params)?;
    sb.apply_pagination(ctx, query_params);

    Ok(sb.build())
}

/// Build a SELECT query for a single record by primary key.
///
/// # Errors
///
/// Returns `AppError::Internal` if the table has no primary key column.
pub fn build_select_one(
    db_ctx: &DatabaseContext,
    ctx: &SelectContext,
    pk_value: &str,
    context: &RequestContext,
) -> Result<BuiltQuery, AppError> {
    let table_config = &db_ctx.table_config;
    let table_name = &table_config.name;
    use crate::db::query::helpers::resolve_fields;

    let fields = resolve_fields(&ctx.fields, table_config);
    let pk_col = find_pk_column(table_config)?;
    let mut sb = SelectBuilder::new(table_name, fields, db_ctx.pool.driver());

    sb.apply_pk_condition(&pk_col, coerce_pk_value(table_config, pk_value));
    sb.apply_where_clause(ctx, context)?;
    sb.limit_one();

    Ok(sb.build())
}

/// Build `SELECT COALESCE(MAX({order_col}), 0) as max_order FROM {table}`.
///
/// Used to determine the next attachment order when adding file/media references.
pub fn build_file_ref_max_order(
    table_name: &str,
    order_col: &str,
    driver: DatabaseDriver,
) -> BuiltQuery {
    let table = quote_identifier(table_name, driver);
    let order = quote_identifier(order_col, driver);
    BuiltQuery {
        sql: format!("SELECT COALESCE(MAX({order}), 0) as max_order FROM {table}"),
        params: Vec::new(),
    }
}

/// Build `INSERT INTO {table} ({columns}) VALUES ({placeholders})`.
///
/// Used for attaching file store entries to content entities.
/// Parameters: file_id, entity_id, content_type, order
pub fn build_file_ref_insert(
    table_name: &str,
    columns: &[&str],
    driver: DatabaseDriver,
) -> BuiltQuery {
    let table = quote_identifier(table_name, driver);
    let col_names: Vec<String> = columns
        .iter()
        .map(|c| quote_identifier(c, driver))
        .collect();
    let placeholders: Vec<String> = (1..=columns.len())
        .map(|i| placeholder(driver, i))
        .collect();

    BuiltQuery {
        sql: format!(
            "INSERT INTO {} ({}) VALUES ({})",
            table,
            col_names.join(", "),
            placeholders.join(", ")
        ),
        params: Vec::new(),
    }
}

/// Used to retrieve a content reference after insertion.
///
/// Shared by media store and file store.
pub fn build_file_ref_select(
    table_name: &str,
    file_id_col: &str,
    entity_id_col: &str,
    content_type_col: &str,
    driver: DatabaseDriver,
) -> BuiltQuery {
    let table = quote_identifier(table_name, driver);
    let fid = quote_identifier(file_id_col, driver);
    let eid = quote_identifier(entity_id_col, driver);
    let ctc = quote_identifier(content_type_col, driver);
    BuiltQuery {
        sql: format!(
            "SELECT * FROM {table} WHERE {fid} = {} AND {eid} = {} AND {ctc} = {}",
            placeholder(driver, 1),
            placeholder(driver, 2),
            placeholder(driver, 3)
        ),
        params: Vec::new(),
    }
}

/// Used for detaching file and media store entries from content entities.
pub fn build_file_ref_delete(
    table_name: &str,
    file_id_col: &str,
    entity_id_col: &str,
    content_type_col: &str,
    driver: DatabaseDriver,
) -> BuiltQuery {
    let table = quote_identifier(table_name, driver);
    let fid = quote_identifier(file_id_col, driver);
    let eid = quote_identifier(entity_id_col, driver);
    let ctc = quote_identifier(content_type_col, driver);
    BuiltQuery {
        sql: format!(
            "DELETE FROM {table} WHERE {fid} = {} AND {eid} = {} AND {ctc} = {}",
            placeholder(driver, 1),
            placeholder(driver, 2),
            placeholder(driver, 3)
        ),
        params: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::listing::SortOrder;
    use crate::{config::types::*, db::query::types::QueryParams};

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

    fn test_crud() -> CrudConfig {
        CrudConfig {
            table: "posts".to_string(),
            database: "main".to_string(),
            fields: vec!["id".to_string(), "title".to_string(), "author".to_string()],
            writable_fields: vec!["title".to_string(), "author".to_string()],
            ..Default::default()
        }
    }

    /// Helper function to create a database context for testing.
    fn test_db_ctx(table: TableConfig, driver: DatabaseDriver) -> DatabaseContext {
        use crate::db::pool::DatabasePool;
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let pool = rt.block_on(async {
            match driver {
                DatabaseDriver::Sqlite => {
                    let options = SqliteConnectOptions::new().filename(":memory:");
                    let pool = SqlitePoolOptions::new()
                        .max_connections(1)
                        .connect_with(options)
                        .await
                        .unwrap();
                    DatabasePool::Sqlite(pool)
                }
                DatabaseDriver::Postgres => {
                    use sqlx::postgres::PgPoolOptions;
                    let url = std::env::var("TEST_POSTGRES_URL").unwrap_or_else(|_| {
                        "postgres://localhost:5432/main_serve_test".to_string()
                    });
                    let pool = PgPoolOptions::new()
                        .max_connections(1)
                        .connect_lazy(&url)
                        .unwrap();
                    DatabasePool::Postgres(pool)
                }
                DatabaseDriver::Mysql => {
                    use sqlx::mysql::MySqlPoolOptions;
                    let url = std::env::var("TEST_MYSQL_URL")
                        .unwrap_or_else(|_| "mysql://localhost:3306/main_serve_test".to_string());
                    let pool = MySqlPoolOptions::new()
                        .max_connections(1)
                        .connect_lazy(&url)
                        .unwrap();
                    DatabasePool::Mysql(pool)
                }
            }
        });

        DatabaseContext {
            pool,
            table_config: table,
        }
    }

    /// Helper function to create a table config with a JSONB column.
    fn test_table_with_jsonb() -> TableConfig {
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
            database: "main".to_string(),
            fields: vec![
                "id".to_string(),
                "title".to_string(),
                "metadata".to_string(),
            ],
            writable_fields: vec!["title".to_string(), "metadata".to_string()],
            filtering: crate::config::types::listing::FilteringConfig {
                allowed_fields: vec!["*".to_string()],
                enabled: true,
            },
            ..Default::default()
        }
    }

    #[test]
    fn test_build_select_list_basic() {
        let table = test_table();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let crud = test_crud();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams::default();
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();
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
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let crud = test_crud();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            filters: [("author".to_string(), "alice".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();
        // Regular columns don't use json_extract, only JSONB nested fields do
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("author"));
        assert_eq!(q.params.len(), 3); // filter value + limit + offset
    }

    #[test]
    fn test_build_select_list_postgres_placeholders() {
        let table = test_table();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Postgres);
        let crud = test_crud();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            filters: [("author".to_string(), "bob".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();
        // Current implementation uses PostgreSQL JSONB operators
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains('$')); // PostgreSQL placeholders
        assert!(!q.params.is_empty());
    }

    #[test]
    fn test_build_select_one() {
        let table = test_table();
        let crud = test_crud();
        let ctx = SelectContext::from(&crud);
        let q = build_select_one(
            &test_db_ctx(table, DatabaseDriver::Sqlite),
            &ctx,
            "42",
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
        let ctx = MutationContext::from(&crud);
        let body = serde_json::json!({"title": "Hello", "author": "Alice"});
        let q = build_insert(
            &test_db_ctx(table, DatabaseDriver::Sqlite),
            &ctx,
            &body,
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
        let ctx = MutationContext::from(&crud);
        let body = serde_json::json!({"title": "Hello", "author": "Alice", "id": 999});
        let q = build_insert(
            &test_db_ctx(table, DatabaseDriver::Sqlite),
            &ctx,
            &body,
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
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let crud = test_crud();
        let ctx = MutationContext::from(&crud);
        let body = serde_json::json!({"title": "Updated"});
        let context = RequestContext::new();
        let q = build_update(&db_ctx, ctx, "42", &body, &context, &None).unwrap();
        assert_eq!(q.sql, "UPDATE posts SET title = ? WHERE id = ?");
        assert_eq!(
            q.params,
            vec![serde_json::json!("Updated"), serde_json::json!(42)]
        );
    }

    #[test]
    fn test_build_delete() {
        let table = test_table();
        let context = RequestContext::new();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let q = build_delete("42", &db_ctx, &context, &None).unwrap();
        assert_eq!(q.sql, "DELETE FROM posts WHERE id = ?");
        assert_eq!(q.params, vec![serde_json::json!(42)]);
    }

    #[test]
    fn test_build_delete_with_where_clause() {
        let table = test_table();
        let context = RequestContext::new();
        let where_clause = Some("author = ${request.user.id}".to_string());
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let q = build_delete("42", &db_ctx, &context, &where_clause).unwrap();
        assert_eq!(
            q.sql,
            "DELETE FROM posts WHERE id = ? AND author = ${request.user.id}"
        );
        assert_eq!(q.params, vec![serde_json::json!(42)]);
    }

    #[test]
    fn test_build_delete_with_interpolated_where_clause() {
        use crate::middleware::auth::extractor::UserInfo;

        let table = test_table();
        let context = RequestContext::new_with_user(UserInfo {
            id: "user-123".to_string(),
            role: None,
        });
        let where_clause = Some("author = ${request.user.id}".to_string());
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let q = build_delete("42", &db_ctx, &context, &where_clause).unwrap();
        assert_eq!(
            q.sql,
            "DELETE FROM posts WHERE id = ? AND author = user-123"
        );
        assert_eq!(q.params, vec![serde_json::json!(42)]);
    }

    #[test]
    fn test_build_update_with_where_clause() {
        let table = test_table();
        let crud = test_crud();
        let ctx = MutationContext::from(&crud);
        let body = serde_json::json!({"title": "Updated"});
        let context = RequestContext::new();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let where_clause = Some("author = ${request.user.id}".to_string());
        let q = build_update(&db_ctx, ctx, "42", &body, &context, &where_clause).unwrap();
        assert_eq!(
            q.sql,
            "UPDATE posts SET title = ? WHERE id = ? AND author = ${request.user.id}"
        );
        assert_eq!(
            q.params,
            vec![serde_json::json!("Updated"), serde_json::json!(42)]
        );
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
        let ctx = MutationContext::from(&crud);
        let body = serde_json::json!({"title": "Hello", "author": "Alice"});
        let q = build_insert(
            &test_db_ctx(table, DatabaseDriver::Postgres),
            &ctx,
            &body,
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
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
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
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

        // Verify the query includes nested JSON extraction for SQLite
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(
            q.sql
                .contains("json_extract(metadata, '$.user.profile.email')")
        );
        assert!(q.sql.contains('='));
        assert!(!q.params.is_empty());
    }

    #[test]
    fn test_filter_lhs_bracket_eq() {
        let table = test_table_with_jsonb();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            // Testing LHS bracket equality: metadata.role[eq]
            filters: [("metadata.role[eq]".to_string(), "admin".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

        // Verify the query includes proper bracket notation conversion to JSON extraction
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("json_extract(metadata, '$.role')"));
        assert!(q.sql.contains('='));
        assert!(!q.params.is_empty());
    }

    #[test]
    fn test_filter_lhs_bracket_gt() {
        let table = test_table_with_jsonb();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            // Testing LHS bracket greater-than: metadata.user.age[gt]
            filters: [("metadata.user.age[gt]".to_string(), "18".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

        // Verify the query includes proper bracket notation conversion to JSON extraction
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("json_extract(metadata, '$.user.age')"));
        assert!(q.sql.contains('>'));
        assert!(!q.params.is_empty());
    }

    #[test]
    fn test_filter_lhs_bracket_lt() {
        let table = test_table_with_jsonb();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            // Testing LHS bracket less-than: metadata.user.age[lte]
            filters: [("metadata.user.age[lte]".to_string(), "65".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

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
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
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
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

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
        let db_ctx = test_db_ctx(table, DatabaseDriver::Postgres);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            sort: Some("metadata.role".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

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
        let db_ctx = test_db_ctx(table, DatabaseDriver::Postgres);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            sort: Some("metadata.role".to_string()),
            order: Some(SortOrder::Desc),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

        // Verify DESC order with JSONB sorting
        assert!(q.sql.contains("DESC"));
        assert!(q.sql.contains("#>>"));
    }

    #[test]
    fn test_sort_jsonb_nested_deep() {
        let table = test_table_with_jsonb();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Postgres);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            sort: Some("metadata.user.profile.age".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

        // Verify nested path sorting
        assert!(q.sql.contains("ORDER BY"));
        assert!(q.sql.contains("{user,profile,age}"));
    }

    #[test]
    fn test_sort_jsonb_mysql() {
        let table = test_table_with_jsonb();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Mysql);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            sort: Some("metadata.role".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

        // Verify MySQL uses JSON_EXTRACT with proper JSONPath syntax
        assert!(q.sql.contains("ORDER BY"));
        assert!(q.sql.contains("JSON_EXTRACT(metadata, '$.role')"));
        assert!(q.sql.contains("ASC"));
    }

    #[test]
    fn test_sort_jsonb_sqlite() {
        let table = test_table_with_jsonb();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            sort: Some("metadata.role".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

        // Verify SQLite uses json_extract with proper JSONPath syntax
        assert!(q.sql.contains("ORDER BY"));
        assert!(q.sql.contains("json_extract(metadata, '$.role')"));
        assert!(q.sql.contains("ASC"));
    }

    #[test]
    fn test_sort_regular_field() {
        let table = test_table();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let crud = test_crud();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            sort: Some("title".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

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
        let db_ctx = test_db_ctx(table, DatabaseDriver::Postgres);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            sort: Some("metadata[role]".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

        // Verify the query includes bracket notation sorting
        assert!(q.sql.contains("ORDER BY"));
        assert!(q.sql.contains("#>>"));
        assert!(q.sql.contains("{role}"));
        assert!(q.sql.contains("posts")); // Correct table name
    }

    #[test]
    fn test_sort_jsonb_nested_bracket_notation() {
        let table = test_table_with_jsonb();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Postgres);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            sort: Some("metadata[user][profile][email]".to_string()),
            order: Some(SortOrder::Desc),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

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
        let db_ctx = test_db_ctx(table, DatabaseDriver::Postgres);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            sort: Some("metadata[user].profile.email".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

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
        let db_ctx = test_db_ctx(table, DatabaseDriver::Postgres);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            filters: [("nonexistent.field".to_string(), "value".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let result = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new());

        // Verify error is returned for nonexistent column
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn test_sort_nonexistent_column_rejected() {
        let table = test_table_with_jsonb();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Postgres);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            sort: Some("nonexistent.field".to_string()),
            order: Some(SortOrder::Asc),
            ..Default::default()
        };
        let result = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new());

        // Verify error is returned for nonexistent sort field
        assert!(result.is_err());
    }

    // ============================================================================
    // JSONB CONTAINS Filter Tests
    // ============================================================================

    #[test]
    fn test_filter_jsonb_contains_postgres() {
        let table = test_table_with_jsonb();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Postgres);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            filters: [("metadata.role[contains]".to_string(), "admin".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

        // PostgreSQL uses @> operator with column name (not table name)
        // For nested paths, extracts with -> before containment check
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("(metadata->'role') @>"));
        assert!(!q.sql.contains("posts @>"));
    }

    #[test]
    fn test_filter_jsonb_contains_sqlite() {
        let table = test_table_with_jsonb();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            filters: [("metadata.tags[contains]".to_string(), "rust".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

        // SQLite uses json_extract equality for nested JSONB paths
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("json_extract(metadata"));
        assert!(q.sql.contains("$.tags"));
        assert!(q.sql.contains('='));
    }

    #[test]
    fn test_filter_jsonb_contains_mysql() {
        let table = test_table_with_jsonb();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Mysql);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            filters: [("metadata.tags[contains]".to_string(), "python".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

        // MySQL uses JSON_CONTAINS for containment checks
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("JSON_CONTAINS"));
        assert!(q.sql.contains("JSON_EXTRACT(metadata, '$.tags')"));
    }

    #[test]
    fn test_filter_non_jsonb_contains() {
        let table = test_table();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
        let crud = test_crud();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            filters: [("author[contains]".to_string(), "al".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

        // Non-JSONB contains generates table.column LIKE '%value%' (substring matching)
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("posts.author LIKE"));
        // Should NOT contain the param as a column name
        assert!(!q.sql.contains("posts.$1 = $1"));
        assert!(!q.sql.contains("posts.? = ?"));
    }

    #[test]
    fn test_filter_non_jsonb_contains_postgres() {
        let table = test_table();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Postgres);
        let crud = test_crud();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            filters: [("title[contains]".to_string(), "Hello".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

        assert!(q.sql.contains("posts.title LIKE"));
    }

    // ============================================================================
    // JSONB EXISTS Filter Tests
    // ============================================================================

    #[test]
    fn test_filter_jsonb_exists_postgres() {
        let table = test_table_with_jsonb();
        let db_ctx = test_db_ctx(table, DatabaseDriver::Postgres);
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            filters: [("metadata.role[exists]".to_string(), String::new())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(&db_ctx, &ctx, &params, &RequestContext::new()).unwrap();

        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("IS NOT NULL"));
        assert!(q.sql.contains("#>>"));
    }

    #[test]
    fn test_filter_jsonb_exists_sqlite() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            filters: [("metadata.status[exists]".to_string(), String::new())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(
            &test_db_ctx(table, DatabaseDriver::Sqlite),
            &ctx,
            &params,
            &RequestContext::new(),
        )
        .unwrap();

        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("IS NOT NULL"));
    }

    #[test]
    fn test_filter_non_jsonb_exists_sqlite() {
        let table = test_table();
        let crud = test_crud();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            filters: [("author[exists]".to_string(), String::new())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(
            &test_db_ctx(table, DatabaseDriver::Sqlite),
            &ctx,
            &params,
            &RequestContext::new(),
        )
        .unwrap();

        // Non-JSONB exists generates table.column IS NOT NULL
        assert!(q.sql.contains("SELECT"));
        assert!(q.sql.contains("posts"));
        assert!(q.sql.contains("posts.author IS NOT NULL"));
        // Should NOT check table name alone
        assert!(!q.sql.contains("posts IS NOT NULL"));
    }

    #[test]
    fn test_filter_non_jsonb_exists_postgres() {
        let table = test_table();
        let crud = test_crud();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            filters: [("title[exists]".to_string(), String::new())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(
            &test_db_ctx(table, DatabaseDriver::Postgres),
            &ctx,
            &params,
            &RequestContext::new(),
        )
        .unwrap();

        assert!(q.sql.contains("posts.title IS NOT NULL"));
    }

    #[test]
    fn test_filter_non_jsonb_exists_mysql() {
        let table = test_table();
        let crud = test_crud();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            filters: [("author[exists]".to_string(), String::new())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let q = build_select_list(
            &test_db_ctx(table, DatabaseDriver::Mysql),
            &ctx,
            &params,
            &RequestContext::new(),
        )
        .unwrap();

        assert!(q.sql.contains("posts.author IS NOT NULL"));
    }

    #[test]
    fn test_filter_nonexistent_jsonb_column_rejected() {
        let table = test_table_with_jsonb();
        let crud = test_crud_with_jsonb_filtering();
        let ctx = SelectContext::from(&crud);
        let params = QueryParams {
            filters: [("other_column.nested".to_string(), "value".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let result = build_select_list(
            &test_db_ctx(table, DatabaseDriver::Postgres),
            &ctx,
            &params,
            &RequestContext::new(),
        );

        // Verify error is returned for nonexistent JSONB column
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }
}

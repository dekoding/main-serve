use super::*;
use crate::config::types::listing::SortOrder;
use crate::config::types::{ColumnType, DatabaseDriver};
use crate::db::query::helpers::is_valid_identifier;
use crate::db::query::types::{MutationContext, SelectContext};
use crate::error::AppError;
use crate::handlers::common::utils::DatabaseContext;
use crate::middleware::auth::extractor::RequestContext;
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
                let url = std::env::var("TEST_POSTGRES_URL")
                    .unwrap_or_else(|_| "postgres://localhost:5432/main_serve_test".to_string());
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
        filters: std::iter::once(("author".to_string(), "alice".to_string())).collect(),
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
        filters: std::iter::once(("author".to_string(), "bob".to_string())).collect(),
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
    let q = build_update(&db_ctx, &ctx, "42", &body, &context, &None).unwrap();
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
    use crate::middleware::auth::extractor::UserInfo;

    let table = test_table();
    let context = RequestContext::new_with_user(UserInfo {
        id: "user-42".to_string(),
        role: None,
    });
    let where_clause = Some("author = ${request.user.id}".to_string());
    let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
    let q = build_delete("42", &db_ctx, &context, &where_clause).unwrap();
    assert_eq!(q.sql, "DELETE FROM posts WHERE id = ? AND author = ?");
    assert_eq!(
        q.params,
        vec![serde_json::json!(42), serde_json::json!("user-42")]
    );
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
    assert_eq!(q.sql, "DELETE FROM posts WHERE id = ? AND author = ?");
    assert_eq!(
        q.params,
        vec![serde_json::json!(42), serde_json::json!("user-123")]
    );
}

#[test]
fn test_build_update_with_where_clause() {
    use crate::middleware::auth::extractor::UserInfo;

    let table = test_table();
    let crud = test_crud();
    let ctx = MutationContext::from(&crud);
    let body = serde_json::json!({"title": "Updated"});
    let context = RequestContext::new_with_user(UserInfo {
        id: "user-42".to_string(),
        role: None,
    });
    let db_ctx = test_db_ctx(table, DatabaseDriver::Sqlite);
    let where_clause = Some("author = ${request.user.id}".to_string());
    let q = build_update(&db_ctx, &ctx, "42", &body, &context, &where_clause).unwrap();
    assert_eq!(
        q.sql,
        "UPDATE posts SET title = ? WHERE id = ? AND author = ?"
    );
    assert_eq!(
        q.params,
        vec![
            serde_json::json!("Updated"),
            serde_json::json!(42),
            serde_json::json!("user-42")
        ]
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
        filters: std::iter::once((
            "metadata.user.profile.email".to_string(),
            "test@example.com".to_string(),
        ))
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
        filters: std::iter::once(("metadata.role[eq]".to_string(), "admin".to_string())).collect(),
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
        filters: std::iter::once(("metadata.user.age[gt]".to_string(), "18".to_string())).collect(),
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
        filters: std::iter::once(("metadata.user.age[lte]".to_string(), "65".to_string()))
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
        filters: std::iter::once(("nonexistent.field".to_string(), "value".to_string())).collect(),
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
        filters: std::iter::once(("metadata.role[contains]".to_string(), "admin".to_string()))
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
        filters: std::iter::once(("metadata.tags[contains]".to_string(), "rust".to_string()))
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
        filters: std::iter::once(("metadata.tags[contains]".to_string(), "python".to_string()))
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
        filters: std::iter::once(("author[contains]".to_string(), "al".to_string())).collect(),
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
        filters: std::iter::once(("title[contains]".to_string(), "Hello".to_string())).collect(),
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
        filters: std::iter::once(("metadata.role[exists]".to_string(), String::new())).collect(),
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
        filters: std::iter::once(("metadata.status[exists]".to_string(), String::new())).collect(),
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
        filters: std::iter::once(("author[exists]".to_string(), String::new())).collect(),
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
        filters: std::iter::once(("title[exists]".to_string(), String::new())).collect(),
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
        filters: std::iter::once(("author[exists]".to_string(), String::new())).collect(),
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
        filters: std::iter::once(("other_column.nested".to_string(), "value".to_string()))
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

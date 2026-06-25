/// Integration tests for dynamic parameter interpolation in SQL queries.
///
/// These tests verify that the interpolation logic correctly resolves values from the request context,
/// including user information, query parameters, and default values. The tests cover interpolation in
/// various CRUD operations, ensuring that the generated SQL queries contain the expected parameters.
mod support;

use main_serve::config::types::ColumnConfig;
use main_serve::config::types::ColumnType;
use main_serve::config::types::CrudConfig;
use main_serve::config::types::DatabaseDriver;
use main_serve::config::types::TableConfig;
use main_serve::db::query::builders::{build_insert, build_select_list, build_update};
use main_serve::db::query::types::QueryParams;
use main_serve::middleware::auth::extractor::RequestContext;

// =============================================================================
// Test Environment Setup
// =============================================================================

fn setup_test_env() -> (TableConfig, CrudConfig) {
    let table = TableConfig {
        name: "posts".to_string(),
        database: "test".to_string(),
        columns: vec![
            ColumnConfig {
                name: "id".to_string(),
                column_type: ColumnType::Integer,
                primary_key: true,
                nullable: false,
                ..Default::default()
            },
            ColumnConfig {
                name: "user_id".to_string(),
                column_type: ColumnType::Integer,
                primary_key: false,
                nullable: false,
                ..Default::default()
            },
            ColumnConfig {
                name: "content".to_string(),
                column_type: ColumnType::Text,
                primary_key: false,
                nullable: false,
                ..Default::default()
            },
            ColumnConfig {
                name: "author_name".to_string(),
                column_type: ColumnType::Varchar,
                primary_key: false,
                nullable: false,
                ..Default::default()
            },
        ],
        foreign_keys: vec![],
    };

    let crud = CrudConfig {
        table: "posts".to_string(),
        database: "test".to_string(),
        fields: vec![
            "id".to_string(),
            "user_id".to_string(),
            "content".to_string(),
            "author_name".to_string(),
        ],
        writable_fields: vec![
            "user_id".to_string(),
            "content".to_string(),
            "author_name".to_string(),
        ],
        where_clause: Some("user_id = ${request.user.id}".to_string()),
        ..Default::default()
    };

    (table, crud)
}

#[tokio::test]
async fn test_interpolation_in_where_clause() {
    let (table, crud) = setup_test_env();
    let context = RequestContext {
        user_id: Some("123".to_string()),
        ..Default::default()
    };

    let q = build_select_list(
        &table,
        &crud,
        &QueryParams::default(),
        DatabaseDriver::Sqlite,
        &context,
    )
    .unwrap();

    assert!(q.sql.contains("WHERE (user_id = ?"));
    assert_eq!(q.params[0], serde_json::json!("123"));
}

#[tokio::test]
async fn test_interpolation_in_insert_body() {
    let (table, crud) = setup_test_env();
    let context = RequestContext {
        user_id: Some("456".to_string()),
        ..Default::default()
    };

    let body = serde_json::json!({
        "user_id": "${request.user.id}",
        "content": "Hello world",
        "author_name": "Admin"
    });

    let q = build_insert(
        &table,
        &crud,
        &body,
        DatabaseDriver::Sqlite,
        &context,
    )
    .unwrap();

    // Note: Sqlite/Mysql order might vary due to BTreeMap in serde_json
    // But we check if "456" is in params.
    assert!(q.params.contains(&serde_json::json!("456")));
    assert!(q.params.contains(&serde_json::json!("Hello world")));
}

#[tokio::test]
async fn test_interpolation_in_update_body() {
    let (table, crud) = setup_test_env();
    let context = RequestContext {
        user_id: Some("789".to_string()),
        ..Default::default()
    };

    let body = serde_json::json!({
        "content": "Updated content by ${request.user.id}"
    });

    let q = build_update(
        &table,
        &crud,
        "1",
        &body,
        DatabaseDriver::Sqlite,
        &context,
        &None,
    )
    .unwrap();

    assert!(
        q.params
            .contains(&serde_json::json!("Updated content by 789"))
    );
}

#[tokio::test]
async fn test_interpolation_with_default_value() {
    let (table, crud) = setup_test_env();
    let context = RequestContext::default(); // user_id is None

    let body = serde_json::json!({
        "author_name": "${request.user.name:-Anonymous}"
    });

    let q = build_insert(
        &table,
        &crud,
        &body,
        DatabaseDriver::Sqlite,
        &context,
    )
    .unwrap();

    assert!(q.params.contains(&serde_json::json!("Anonymous")));
}

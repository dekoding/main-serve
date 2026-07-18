/// JSON schema validation integration tests.
///
/// Tests run against a real database (SQLite by default) via `TestDatabase`.
/// Covers all three schema mechanisms (inline, external file, global ref)
/// and verifies validation runs at the CRUD handler level.
mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use support::json_body;
use tower::ServiceExt;

use crate::support::configs::json_schema_configs::{
    JSONB_EXTERNAL_SCHEMA_CONFIG, JSONB_GLOBAL_REF_SCHEMA_CONFIG, JSONB_INLINE_SCHEMA_CONFIG,
    JSONB_MULTIPLE_COLUMNS_CONFIG, JSONB_NO_SCHEMA_CONFIG,
};
use crate::support::db::TestDatabase;
use crate::support::db::enabled_backends;

/// POST a JSON body to `/api/posts` and return the response.
async fn post_posts(app: &axum::Router, body: serde_json::Value) -> axum::http::Response<Body> {
    let req = Request::builder()
        .method("POST")
        .uri("/api/posts")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    app.clone().oneshot(req).await.unwrap()
}

/// PUT a JSON body to `/api/posts/{id}` and return the response.
async fn put_posts(
    app: &axum::Router,
    id: i64,
    body: serde_json::Value,
) -> axum::http::Response<Body> {
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/posts/{id}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    app.clone().oneshot(req).await.unwrap()
}

// =========================================================================
// Inline schema validation
// =========================================================================

#[tokio::test]
async fn test_create_with_valid_jsonb_data() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_inline_valid");
        let (app, _state) = test_db
            .setup_app(JSONB_INLINE_SCHEMA_CONFIG, "jsonb_inline_valid.yaml")
            .await;

        let response = post_posts(
            &app,
            serde_json::json!({
                "title": "Valid Post",
                "metadata": {
                    "role": "admin",
                    "status": "active"
                }
            }),
        )
        .await;

        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "valid JSONB data matching schema should succeed, backend: {backend}"
        );
    }
}

#[tokio::test]
async fn test_create_with_invalid_jsonb_data_missing_required() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_inline_missing");
        let (app, _state) = test_db
            .setup_app(JSONB_INLINE_SCHEMA_CONFIG, "jsonb_inline_missing.yaml")
            .await;

        let response = post_posts(
            &app,
            serde_json::json!({
                "title": "Invalid Post",
                "metadata": {
                    "extra_field": "this should be rejected"
                }
            }),
        )
        .await;

        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "JSONB data with additional properties should fail validation, backend: {backend}"
        );
        let body = json_body(response).await;
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("JSON Schema validation failed"),
            "error message should mention JSON Schema validation, got: {body}"
        );
    }
}

#[tokio::test]
async fn test_create_with_invalid_jsonb_data_wrong_type() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_inline_wrong_type");
        let (app, _state) = test_db
            .setup_app(JSONB_INLINE_SCHEMA_CONFIG, "jsonb_inline_wrong_type.yaml")
            .await;

        let response = post_posts(
            &app,
            serde_json::json!({
                "title": "Invalid Post",
                "metadata": {
                    "role": 123
                }
            }),
        )
        .await;

        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "JSONB data with wrong type should fail validation, backend: {backend}"
        );
        let body = json_body(response).await;
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("JSON Schema validation failed"),
            "error should mention JSON Schema validation, got: {body}"
        );
    }
}

// =========================================================================
// Update (PUT) validation
// =========================================================================

#[tokio::test]
async fn test_update_with_valid_jsonb_data() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_update_valid");
        let (app, _state) = test_db
            .setup_app(JSONB_INLINE_SCHEMA_CONFIG, "jsonb_update_valid.yaml")
            .await;

        // Create a record first
        let create_resp = post_posts(
            &app,
            serde_json::json!({
                "title": "Original Title",
                "metadata": {
                    "role": "editor",
                    "status": "draft"
                }
            }),
        )
        .await;
        assert_eq!(
            create_resp.status(),
            StatusCode::CREATED,
            "create should succeed, backend: {backend}"
        );
        let created = json_body(create_resp).await;
        let id = created["data"]["id"].as_i64().unwrap();

        // Update with valid data
        let update_resp = put_posts(
            &app,
            id,
            serde_json::json!({
                "title": "Updated Title",
                "metadata": {
                    "role": "admin"
                }
            }),
        )
        .await;

        assert_eq!(
            update_resp.status(),
            StatusCode::OK,
            "valid update should succeed, backend: {backend}"
        );
    }
}

#[tokio::test]
async fn test_update_with_invalid_jsonb_data() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_update_invalid");
        let (app, _state) = test_db
            .setup_app(JSONB_INLINE_SCHEMA_CONFIG, "jsonb_update_invalid.yaml")
            .await;

        // Create a record first
        let create_resp = post_posts(
            &app,
            serde_json::json!({
                "title": "Original",
                "metadata": {
                    "role": "admin"
                }
            }),
        )
        .await;
        assert_eq!(
            create_resp.status(),
            StatusCode::CREATED,
            "create should succeed, backend: {backend}"
        );
        let created = json_body(create_resp).await;
        let id = created["data"]["id"].as_i64().unwrap();

        // Update with invalid data (additional properties)
        let update_resp = put_posts(
            &app,
            id,
            serde_json::json!({
                "title": "Updated",
                "metadata": {
                    "role": "admin",
                    "forbidden": true
                }
            }),
        )
        .await;

        assert_eq!(
            update_resp.status(),
            StatusCode::BAD_REQUEST,
            "invalid update should fail, backend: {backend}"
        );
        let body = json_body(update_resp).await;
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("JSON Schema validation failed"),
            "error should mention JSON Schema validation, got: {body}"
        );
    }
}

#[tokio::test]
async fn test_update_partial_partial_valid_fields_other_valid() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_update_partial");
        let (app, _state) = test_db
            .setup_app(JSONB_INLINE_SCHEMA_CONFIG, "jsonb_update_partial.yaml")
            .await;

        // Create a record first with valid metadata
        let create_resp = post_posts(
            &app,
            serde_json::json!({
                "title": "Original",
                "metadata": {
                    "role": "admin",
                    "status": "active"
                }
            }),
        )
        .await;
        assert_eq!(
            create_resp.status(),
            StatusCode::CREATED,
            "create should succeed, backend: {backend}"
        );
        let created = json_body(create_resp).await;
        let id = created["data"]["id"].as_i64().unwrap();

        // Update only the title (no metadata in body) - should succeed
        // because only fields present in the body are validated
        let update_resp = put_posts(
            &app,
            id,
            serde_json::json!({
                "title": "Just updating title"
            }),
        )
        .await;

        assert_eq!(
            update_resp.status(),
            StatusCode::OK,
            "partial update without metadata should succeed, backend: {backend}"
        );
    }
}

// =========================================================================
// No schema validation (control test)
// =========================================================================

#[tokio::test]
async fn test_create_without_schema_validation() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_no_schema");
        let (app, _state) = test_db
            .setup_app(JSONB_NO_SCHEMA_CONFIG, "jsonb_no_schema.yaml")
            .await;

        let response = post_posts(
            &app,
            serde_json::json!({
                "title": "Any Data",
                "metadata": {
                    "completely": "arbitrary",
                    "data": [1, 2, 3],
                    "nested": {
                        "anything": true
                    }
                }
            }),
        )
        .await;

        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "data without schema should always be accepted, backend: {backend}"
        );
    }
}

// =========================================================================
// External schema file validation
// =========================================================================

#[tokio::test]
async fn test_external_schema_file_validation() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_external");
        let schema_content = r#"{"type": "object", "required": ["author_id"], "properties": {"author_id": {"type": "integer"}}}
"#;
        std::fs::write(
            test_db.root_dir.path().join("jsonb_validation_schema.json"),
            schema_content,
        )
        .expect("write schema file");

        let (app, _state) = test_db
            .setup_app(JSONB_EXTERNAL_SCHEMA_CONFIG, "jsonb_external.yaml")
            .await;

        // Valid data
        let valid_resp = post_posts(
            &app,
            serde_json::json!({
                "title": "External Schema Post",
                "metadata": {
                    "author_id": 42
                }
            }),
        )
        .await;
        assert_eq!(
            valid_resp.status(),
            StatusCode::CREATED,
            "data matching external schema should succeed, backend: {backend}"
        );

        // Invalid data - missing required field
        let invalid_resp = post_posts(
            &app,
            serde_json::json!({
                "title": "Invalid Post",
                "metadata": {
                    "some_other_field": "no author_id"
                }
            }),
        )
        .await;
        assert_eq!(
            invalid_resp.status(),
            StatusCode::BAD_REQUEST,
            "data not matching external schema should fail, backend: {backend}"
        );
    }
}

// =========================================================================
// Global schema reference validation
// =========================================================================

#[tokio::test]
async fn test_global_schema_reference_validation() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_global_ref");
        let (app, _state) = test_db
            .setup_app(JSONB_GLOBAL_REF_SCHEMA_CONFIG, "jsonb_global_ref.yaml")
            .await;

        // Valid data matching the global schema (requires name and email)
        let valid_resp = post_posts(
            &app,
            serde_json::json!({
                "title": "Global Ref Post",
                "user_info": {
                    "name": "Alice",
                    "email": "alice@example.com"
                }
            }),
        )
        .await;
        assert_eq!(
            valid_resp.status(),
            StatusCode::CREATED,
            "data matching global schema should succeed, backend: {backend}"
        );

        // Invalid data - missing required 'email' field
        let invalid_resp = post_posts(
            &app,
            serde_json::json!({
                "title": "Invalid Global Ref Post",
                "user_info": {
                    "name": "Bob"
                }
            }),
        )
        .await;
        assert_eq!(
            invalid_resp.status(),
            StatusCode::BAD_REQUEST,
            "data missing required field from global schema should fail, backend: {backend}"
        );
        let body = json_body(invalid_resp).await;
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("JSON Schema validation failed"),
            "error should mention JSON Schema validation, got: {body}"
        );
    }
}

// =========================================================================
// Multiple columns with multiple schemas
// =========================================================================

#[tokio::test]
async fn test_multiple_columns_multiple_schemas() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_multi_columns");
        let (app, _state) = test_db
            .setup_app(JSONB_MULTIPLE_COLUMNS_CONFIG, "jsonb_multi_columns.yaml")
            .await;

        // Valid data for both columns
        let valid_resp = post_posts(
            &app,
            serde_json::json!({
                "title": "Multi Schema Post",
                "metadata": {
                    "role": "admin"
                },
                "tags": ["rust", "jsonb"]
            }),
        )
        .await;
        assert_eq!(
            valid_resp.status(),
            StatusCode::CREATED,
            "valid data for all columns should succeed, backend: {backend}"
        );

        // Invalid metadata (missing required 'role')
        let invalid_meta_resp = post_posts(
            &app,
            serde_json::json!({
                "title": "Bad Metadata",
                "metadata": {
                    "extra": "field"
                },
                "tags": ["rust"]
            }),
        )
        .await;
        assert_eq!(
            invalid_meta_resp.status(),
            StatusCode::BAD_REQUEST,
            "invalid metadata should fail, backend: {backend}"
        );

        // Invalid tags (not an array of strings)
        let invalid_tags_resp = post_posts(
            &app,
            serde_json::json!({
                "title": "Bad Tags",
                "metadata": {
                    "role": "user"
                },
                "tags": "not an array"
            }),
        )
        .await;
        assert_eq!(
            invalid_tags_resp.status(),
            StatusCode::BAD_REQUEST,
            "invalid tags should fail, backend: {backend}"
        );
    }
}

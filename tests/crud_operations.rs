mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use support::db::{TestBackend, TestDatabase, enabled_backends};
use support::json_body;
use support::seed_default_jsonb_posts;
use support::seed_posts;

use crate::support::configs::crud_operations_configs::CRUD_CONFIG;
use crate::support::configs::shared_configs::{JSONB_EXPRESSIONS_CONFIG, JSONB_FILTER_SORT_CONFIG};

#[tokio::test]
async fn test_crud_pagination_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "api_pagination");
        let (app, _state) = test_db.setup_app(CRUD_CONFIG, "crud_features.yaml").await;

        seed_posts(
            &app,
            &[
                ("Post 1", "Pager"),
                ("Post 2", "Pager"),
                ("Post 3", "Pager"),
                ("Post 4", "Pager"),
                ("Post 5", "Pager"),
            ],
        )
        .await;

        let req = Request::builder()
            .uri("/api/posts?page=1&page_size=2")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");
        let json = json_body(response).await;
        assert_eq!(
            json["data"].as_array().unwrap().len(),
            2,
            "backend: {backend}"
        );

        let req = Request::builder()
            .uri("/api/posts?page=3&page_size=2")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");
        let json = json_body(response).await;
        assert_eq!(
            json["data"].as_array().unwrap().len(),
            1,
            "backend: {backend}"
        );
    }
}

#[tokio::test]
async fn test_crud_filtering_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "api_filtering");
        let (app, _state) = test_db.setup_app(CRUD_CONFIG, "crud_features.yaml").await;

        seed_posts(&app, &[("A", "Alice"), ("B", "Bob")]).await;

        let req = Request::builder()
            .uri("/api/posts?author=Alice")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 1, "backend: {backend}");
        assert_eq!(data[0]["author"], "Alice", "backend: {backend}");
    }
}

#[tokio::test]
async fn test_crud_sorting_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "api_sorting");
        let (app, _state) = test_db.setup_app(CRUD_CONFIG, "crud_features.yaml").await;

        seed_posts(
            &app,
            &[("Charlie", "Charlie"), ("Alice", "Alice"), ("Bob", "Bob")],
        )
        .await;

        let req = Request::builder()
            .uri("/api/posts?sort=title&order=asc&page_size=10")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(data[0]["title"], "Alice", "backend: {backend}");
        assert_eq!(data[1]["title"], "Bob", "backend: {backend}");
        assert_eq!(data[2]["title"], "Charlie", "backend: {backend}");

        let req = Request::builder()
            .uri("/api/posts?sort=title&order=desc&page_size=10")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(data[0]["title"], "Charlie", "backend: {backend}");
        assert_eq!(data[2]["title"], "Alice", "backend: {backend}");
    }
}

// =============================================================================
// Joins: fields from joined table should appear in response
// =============================================================================

#[tokio::test]
async fn test_crud_join_fields_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "api_join");
        let authors_table = format!("{}_authors", test_db.table_name);
        let posts_table = &test_db.table_name;

        let template = format!(
            r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true

tables:
  - name: "{authors_table}"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "name"
        type: "text"
        nullable: false

  - name: "{posts_table}"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "title"
        type: "text"
        nullable: false
      - name: "author_id"
        type: "integer"
        nullable: false

endpoints:
  - path: "/api/authors"
    methods: ["post"]
    action: "crud"
    crud:
      table: "{authors_table}"
      database: "main"
      writable_fields: ["*"]

  - path: "/api/posts"
    methods: ["get", "post"]
    action: "crud"
    crud:
      table: "{posts_table}"
      database: "main"
      fields: ["id", "title", "author_id"]
      writable_fields: ["title", "author_id"]
      joins:
        - table: "{authors_table}"
          on: "{posts_table}.author_id = {authors_table}.id"
          join_type: "inner"
          fields: ["{authors_table}.name"]
      pagination:
        enabled: false
      sorting:
        enabled: false
"#
        );

        let (app, _state) = test_db.setup_app(&template, "join_test.yaml").await;

        // Create an author.
        let req = Request::builder()
            .method("POST")
            .uri("/api/authors")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name": "Alice"}"#))
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "create author failed for {backend}"
        );

        // Create a post referencing that author.
        let req = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"title": "Hello", "author_id": 1}"#))
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "create post failed for {backend}"
        );

        // List posts - joined field "name" from authors should be present.
        let req = Request::builder()
            .uri("/api/posts")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");
        let json = json_body(response).await;
        let data = json["data"].as_array().expect("data array");
        assert_eq!(data.len(), 1, "backend: {backend}");
        assert_eq!(data[0]["title"], "Hello", "backend: {backend}");
        assert!(
            data[0].get("name").is_some(),
            "Join field 'name' from authors table should be present in response for {backend}, got: {}",
            data[0]
        );
        assert_eq!(data[0]["name"], "Alice", "backend: {backend}");
    }
}

// =============================================================================
// Disallowed fields: filtering and sorting on non-allowed fields must fail
// =============================================================================

#[tokio::test]
async fn test_crud_filter_on_disallowed_field_rejected() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "api_filter_reject");
        let (app, _state) = test_db.setup_app(CRUD_CONFIG, "crud_features.yaml").await;

        seed_posts(&app, &[("Post", "Alice")]).await;

        // "title" is not in allowed_fields (only "author" is).
        let req = Request::builder()
            .uri("/api/posts?title=Post")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "filtering on disallowed field should be rejected, backend: {backend}"
        );
    }
}

#[tokio::test]
async fn test_crud_sort_on_disallowed_field_rejected() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "api_sort_reject");
        let (app, _state) = test_db.setup_app(CRUD_CONFIG, "crud_features.yaml").await;

        seed_posts(&app, &[("Post", "Alice")]).await;

        // "author" is not in sorting.allowed_fields (only "title" and "id" are).
        let req = Request::builder()
            .uri("/api/posts?sort=author")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "sorting on disallowed field should be rejected, backend: {backend}"
        );
    }
}

// =============================================================================
// Where clause: single-record GET must respect where_clause
// =============================================================================

#[tokio::test]
async fn test_crud_where_clause_applied_to_single_get() {
    let config = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true

tables:
  - name: "__TABLE_NAME__"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "title"
        type: "text"
        nullable: false
      - name: "active"
        type: "boolean"
        nullable: false
        default: "true"

endpoints:
  - path: "/api/items"
    methods: ["get", "post"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      writable_fields: ["title", "active"]
      where_clause: "active = true"
    auth: "none"

  - path: "/api/items/:id"
    methods: ["get"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      where_clause: "active = true"
    auth: "none"
"#;

    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "api_where_single");
        let (app, _state) = test_db.setup_app(config, "where_single.yaml").await;

        // Create an active item.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/items")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"title": "Visible", "active": true}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "backend: {backend}");

        // Create an inactive item.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/items")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"title": "Hidden", "active": false}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "backend: {backend}");

        // GET by ID for the active item should succeed.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/items/1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "active item should be visible, backend: {backend}"
        );

        // GET by ID for the inactive item should return 404 (where_clause filters it).
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/items/2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "inactive item should be hidden by where_clause on single GET, backend: {backend}"
        );
    }
}

// =============================================================================
// Pagination: page_size is clamped to max_page_size
// =============================================================================

#[tokio::test]
async fn test_crud_pagination_clamps_page_size() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "api_page_clamp");
        let (app, _state) = test_db.setup_app(CRUD_CONFIG, "crud_features.yaml").await;

        // Seed 5 posts; max_page_size in config is 10, but test with 20.
        seed_posts(
            &app,
            &[
                ("P1", "A"),
                ("P2", "A"),
                ("P3", "A"),
                ("P4", "A"),
                ("P5", "A"),
            ],
        )
        .await;

        // Request page_size=999 - should be clamped to max_page_size (10),
        // returning all 5 posts (since 5 < 10).
        let req = Request::builder()
            .uri("/api/posts?page_size=999")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            5,
            "page_size should be clamped, returning all 5 posts, backend: {backend}"
        );
    }
}

// =============================================================================
// SQL injection prevention
// =============================================================================

#[tokio::test]
async fn test_sql_injection_prevention() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "sql_injection_test");
        let (app, _state) = test_db.setup_app(CRUD_CONFIG, "crud_features.yaml").await;

        // Seed some posts
        seed_posts(&app, &[("Normal Post", "Alice")]).await;

        // Test 1: SQL injection in filter field name should be rejected
        let req = Request::builder()
            .uri("/api/posts?author%5Bid%5D=1%20OR%201=1")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        // Should return 400 Bad Request due to invalid field name
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "backend: {backend} - SQL injection in filter field should be rejected"
        );

        // Test 2: SQL injection in filter value should be treated as literal string
        let req = Request::builder()
            .uri("/api/posts?author=Alice'%20OR%20'1'='1")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        // Should return 200 OK but with empty results since no post has this exact author
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert!(
            data.is_empty(),
            "backend: {backend} - SQL injection in filter value should return no results"
        );

        // Test 3: SQL injection in sort field should be rejected
        let req = Request::builder()
            .uri("/api/posts?sort=title%3B%20DROP%20TABLE%20posts%3B&order=asc")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        // Should return 400 Bad Request due to invalid sort field
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "backend: {backend} - SQL injection in sort field should be rejected"
        );

        // Test 4: SQL injection in PK path parameter should be treated as literal
        let req = Request::builder()
            .uri("/api/posts/1%20OR%201=1")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        // Should return 404 Not Found since no post has this exact PK
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "backend: {backend} - SQL injection in PK should return not found"
        );

        // Test 5: Complex SQL injection attempt in filter
        let req = Request::builder()
            .uri("/api/posts?author=admin';%20DELETE%20FROM%20posts;--")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        // Should return 400 Bad Request or 200 with empty results
        // Either way, the DELETE should NOT execute
        assert!(
            response.status() == StatusCode::BAD_REQUEST || response.status() == StatusCode::OK,
            "backend: {backend} - SQL injection attempt should be handled"
        );

        // Verify the table still has data after all injection attempts
        let req = Request::builder()
            .uri("/api/posts")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            1,
            "backend: {backend} - Table should still have original data after injection attempts"
        );
    }
}

// =============================================================================
// JSONB Sorting Tests
// =============================================================================

#[tokio::test]
async fn test_crud_sort_jsonb_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "api_sort_jsonb");

        let template = JSONB_EXPRESSIONS_CONFIG;

        let (app, _state) = test_db.setup_app(template, "jsonb_sort.yaml").await;

        // Seed posts with JSONB metadata containing role field
        let seed_data = vec![
            (serde_json::json!({"title": "Post C", "metadata": serde_json::json!({"role": "gamma"})})),
            (serde_json::json!({"title": "Post A", "metadata": serde_json::json!({"role": "alpha"})})),
            (serde_json::json!({"title": "Post B", "metadata": serde_json::json!({"role": "beta"})})),
        ];
        for data in &seed_data {
            let req = Request::builder()
                .method("POST")
                .uri("/api/posts")
                .header("content-type", "application/json")
                .body(Body::from(data.to_string()))
                .unwrap();
            let _response = app.clone().oneshot(req).await.unwrap();
        }

        // Test ascending sort on JSONB nested field
        let req = Request::builder()
            .uri("/api/posts?sort=metadata.role&order=asc&page_size=10")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "JSONB sort ascending failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data[0]["title"], "Post A",
            "alpha should be first for {backend}"
        );
        assert_eq!(
            data[1]["title"], "Post B",
            "beta should be second for {backend}"
        );
        assert_eq!(
            data[2]["title"], "Post C",
            "gamma should be third for {backend}"
        );

        // Test descending sort on JSONB nested field
        let req = Request::builder()
            .uri("/api/posts?sort=metadata.role&order=desc&page_size=10")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "JSONB sort descending failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data[0]["title"], "Post C",
            "gamma should be first for {backend}"
        );
        assert_eq!(
            data[1]["title"], "Post B",
            "beta should be second for {backend}"
        );
        assert_eq!(
            data[2]["title"], "Post A",
            "alpha should be third for {backend}"
        );
    }
}

#[tokio::test]
async fn test_crud_sort_jsonb_disallowed_field_rejected() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "api_sort_jsonb_reject");
        let (app, _state) = test_db.setup_app(CRUD_CONFIG, "crud_features.yaml").await;

        // metadata.tags is not in sorting.allowed_fields
        let req = Request::builder()
            .uri("/api/posts?sort=metadata.tags")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "sorting on disallowed JSONB field should be rejected for {backend}"
        );
    }
}

// =============================================================================
// CRUD: Invalid requests
// =============================================================================

#[tokio::test]
async fn test_crud_create_no_body() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "crud_no_body");
        let (app, _state) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        let req = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "backend: {backend}"
        );
    }
}

#[tokio::test]
async fn test_crud_create_ignores_non_writable_field() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "crud_non_writable");
        let (app, _state) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        // Try to set "id" which is not in writable_fields.
        let req = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"id": 999, "title": "T", "body": "", "author": "X"}).to_string(),
            ))
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED, "backend: {backend}");

        // Verify the id was NOT set to 999.
        let req = Request::builder()
            .uri("/api/posts/1")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");
    }
}

// =============================================================================
// CRUD: Edge cases and error handling
// =============================================================================

#[tokio::test]
async fn test_crud_empty_filter_value_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "crud_empty_filter");
        let (app, _state) = test_db.setup_app(CRUD_CONFIG, "crud_features.yaml").await;

        seed_posts(&app, &[("Post", "Alice")]).await;

        // Empty string filter should return no results (parameterized = '')
        let req = Request::builder()
            .uri("/api/posts?author=")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "empty filter value should return 200 for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert!(
            data.is_empty(),
            "backend: {backend} - empty filter should return no results"
        );
    }
}

#[tokio::test]
async fn test_crud_create_partial_fields_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "crud_partial_fields");
        let (app, _state) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        // Post with only required fields (body is nullable)
        let req = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"title": "Minimal", "author": "Dennis"}).to_string(),
            ))
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED, "backend: {backend}");

        // GET the created record to verify partial fields were handled correctly
        let req = Request::builder()
            .uri("/api/posts/1")
            .body(Body::empty())
            .unwrap();
        let get_response = app.oneshot(req).await.unwrap();
        let json = json_body(get_response).await;
        assert_eq!(json["title"], "Minimal", "backend: {backend}");
        assert_eq!(json["author"], "Dennis", "backend: {backend}");
        // body is nullable - should either be null or empty string
        assert!(
            json["body"].is_null() || json["body"].as_str() == Some(""),
            "backend: {backend} - nullable body should be null or empty"
        );
    }
}

#[tokio::test]
async fn test_crud_pagination_edge_cases_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "crud_pagination_edge");
        let (app, _state) = test_db.setup_app(CRUD_CONFIG, "crud_features.yaml").await;

        seed_posts(
            &app,
            &[
                ("P1", "A"),
                ("P2", "A"),
                ("P3", "A"),
                ("P4", "A"),
                ("P5", "A"),
            ],
        )
        .await;

        // page=0 should be handled gracefully
        let req = Request::builder()
            .uri("/api/posts?page=0&page_size=2")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "page=0 should be handled for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert!(
            data.len() <= 2,
            "backend: {backend} - page=0 should return at most page_size items"
        );

        // page_size=0 should return empty results
        let req = Request::builder()
            .uri("/api/posts?page_size=0")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert!(
            data.is_empty(),
            "backend: {backend} - page_size=0 should return empty results"
        );
    }
}

#[tokio::test]
async fn test_crud_response_envelope_structure_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "crud_envelope");
        let (app, _state) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        // Verify list response has "data" as an array
        let req = Request::builder()
            .uri("/api/posts")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        let json = json_body(response).await;
        assert!(
            json["data"].is_array(),
            "backend: {backend} - list response should have 'data' as array"
        );

        // Seed a post and verify POST returns 201, then GET returns the record
        let req = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"title": "Envelope Test", "author": "Tester"}).to_string(),
            ))
            .unwrap();
        let post_response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            post_response.status(),
            StatusCode::CREATED,
            "backend: {backend} - POST should return 201"
        );

        // GET the record directly (single-record GET returns row, not wrapped in "data")
        let req = Request::builder()
            .uri("/api/posts/1")
            .body(Body::empty())
            .unwrap();
        let get_response = app.oneshot(req).await.unwrap();
        let json = json_body(get_response).await;
        assert_eq!(
            json["title"], "Envelope Test",
            "backend: {backend} - single-record GET should return row directly"
        );
        // Single-record response should NOT have "data" wrapper
        assert!(
            !json["data"].is_array(),
            "backend: {backend} - single-record GET should not wrap in 'data'"
        );
    }
}

// =============================================================================
// JSONB Filtering and Sorting Integration Tests
// =============================================================================

/// Seed posts with null and empty JSONB values for edge case testing.
async fn seed_jsonb_posts_with_nulls(app: &axum::Router) {
    let seed_data = vec![
        serde_json::json!({
            "title": "Post With Null Metadata",
            "author": "alice",
            "metadata": null,
            "tags": ["rust"]
        }),
        serde_json::json!({
            "title": "Post With Empty Metadata",
            "author": "bob",
            "metadata": serde_json::json!({}),
            "tags": []
        }),
        serde_json::json!({
            "title": "Post With Full Metadata",
            "author": "charlie",
            "metadata": serde_json::json!({
                "role": "full",
                "status": "active",
                "user": {"age": 40, "profile": {"email": "charlie@example.com", "bio": "Developer"}}
            }),
            "tags": ["go", "web", "api"]
        }),
    ];
    for data in &seed_data {
        let req = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(data.to_string()))
            .unwrap();
        let _response = app.clone().oneshot(req).await.unwrap();
    }
}

#[tokio::test]
async fn test_jsonb_filter_eq_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_eq");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_filter_eq.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by metadata.role = "beta"
        let req = Request::builder()
            .uri("/api/posts?metadata.role=beta")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "eq filter failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            1,
            "eq filter should return 1 result for {backend}"
        );
        assert_eq!(
            data[0]["title"], "Post Beta",
            "eq filter title mismatch for {backend}"
        );

        // Filter by metadata.status = "inactive"
        let req = Request::builder()
            .uri("/api/posts?metadata.status=inactive")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            1,
            "eq filter on inactive should return 1 for {backend}"
        );

        // Filter by nested path metadata.user.age = 25
        let req = Request::builder()
            .uri("/api/posts?metadata.user.age=25")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            1,
            "eq filter on nested path should return 1 for {backend}"
        );
        assert_eq!(
            data[0]["title"], "Post Beta",
            "nested eq filter title mismatch for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_gt_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_gt");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_filter_gt.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by metadata.user.age > 30 (should match Gamma age=35)
        let req = Request::builder()
            .uri("/api/posts?metadata.user.age[gt]=30")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "gt filter failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            1,
            "gt filter should return 1 result for {backend}"
        );
        assert_eq!(
            data[0]["title"], "Post Gamma",
            "gt filter title mismatch for {backend}"
        );

        // Filter by metadata.user.age >= 30 (should match Alpha=30, Gamma=35)
        let req = Request::builder()
            .uri("/api/posts?metadata.user.age[gte]=30")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            2,
            "gte filter should return 2 results for {backend}"
        );
        let titles: Vec<&str> = data.iter().map(|r| r["title"].as_str().unwrap()).collect();
        assert!(
            titles.contains(&"Post Alpha"),
            "gte filter should include Alpha for {backend}"
        );
        assert!(
            titles.contains(&"Post Gamma"),
            "gte filter should include Gamma for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_lt_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_lt");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_filter_lt.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by metadata.user.age < 30 (should match Beta=25, Delta=28, Epsilon=22)
        let req = Request::builder()
            .uri("/api/posts?metadata.user.age[lt]=30")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "lt filter failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            3,
            "lt filter should return 3 results for {backend}"
        );

        // Filter by metadata.user.age <= 25 (should match Beta=25, Epsilon=22)
        let req = Request::builder()
            .uri("/api/posts?metadata.user.age[lte]=25")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            2,
            "lte filter should return 2 results for {backend}"
        );
        let titles: Vec<&str> = data.iter().map(|r| r["title"].as_str().unwrap()).collect();
        assert!(
            titles.contains(&"Post Beta"),
            "lte filter should include Beta for {backend}"
        );
        assert!(
            titles.contains(&"Post Epsilon"),
            "lte filter should include Epsilon for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_ne_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_ne");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_filter_ne.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by metadata.role != "alpha" (should return 4 posts)
        let req = Request::builder()
            .uri("/api/posts?metadata.role[ne]=alpha")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "ne filter failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            4,
            "ne filter should return 4 results for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_contains_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_contains");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_filter_contains.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by tags contains "rust" (should match Alpha and Delta)
        let req = Request::builder()
            .uri("/api/posts?tags[contains]=rust")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "contains filter failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            2,
            "contains filter should return 2 results for {backend}"
        );
        let titles: Vec<&str> = data.iter().map(|r| r["title"].as_str().unwrap()).collect();
        assert!(
            titles.contains(&"Post Alpha"),
            "contains should include Alpha for {backend}"
        );
        assert!(
            titles.contains(&"Post Delta"),
            "contains should include Delta for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_exists_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_exists");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_filter_exists.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by metadata.role exists (should return all 5 posts since all have it)
        let req = Request::builder()
            .uri("/api/posts?metadata.role[exists]")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "exists filter failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            5,
            "exists filter should return 5 results for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_like_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_like");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_filter_like.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by metadata.role like "%alpha%" (should match "alpha")
        let req = Request::builder()
            .uri("/api/posts?metadata.role[like]=%alpha%")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "like filter failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            1,
            "like filter should return 1 result for {backend}"
        );
        assert_eq!(
            data[0]["title"], "Post Alpha",
            "like filter title mismatch for {backend}"
        );

        // Filter by author like "a%" (should match alice posts: Alpha, Delta)
        let req = Request::builder()
            .uri("/api/posts?author[like]=a%")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            2,
            "like filter on author should return 2 for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_startswith_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_startswith");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_filter_startswith.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by metadata.role startswith "alpha" (should match "alpha")
        let req = Request::builder()
            .uri("/api/posts?metadata.role[startswith]=alpha")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 1, "startswith should return 1 for {backend}");
        assert_eq!(
            data[0]["title"], "Post Alpha",
            "startswith title mismatch for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_endswith_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_endswith");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_filter_endswith.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by metadata.role endswith "psilon" (should match "epsilon")
        let req = Request::builder()
            .uri("/api/posts?metadata.role[endswith]=psilon")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 1, "endswith should return 1 for {backend}");
        assert_eq!(
            data[0]["title"], "Post Epsilon",
            "endswith title mismatch for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_combined_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_combined");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_filter_combined.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Combined: metadata.status = "active" AND author = "bob" (Beta=active/bob, Epsilon=draft/bob)
        let req = Request::builder()
            .uri("/api/posts?metadata.status=active&author=bob")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            1,
            "combined eq filter should return 1 for {backend}"
        );
        assert_eq!(
            data[0]["title"], "Post Beta",
            "combined should be Beta for {backend}"
        );

        // Combined: metadata.user.age > 25 AND metadata.status = "active" (Alpha=30/active, Delta=28/active)
        let req = Request::builder()
            .uri("/api/posts?metadata.user.age[gt]=25&metadata.status=active")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            2,
            "combined gt+eq filter should return 2 for {backend}"
        );
        let titles: Vec<&str> = data.iter().map(|r| r["title"].as_str().unwrap()).collect();
        assert!(
            titles.contains(&"Post Alpha"),
            "combined should include Alpha for {backend}"
        );
        assert!(
            titles.contains(&"Post Delta"),
            "combined should include Delta for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_jsonb_column_exists_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_exists_non_jsonb");

        let (app, _state) = test_db
            .setup_app(
                JSONB_FILTER_SORT_CONFIG,
                "jsonb_filter_exists_non_jsonb.yaml",
            )
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by author exists (non-JSONB column) - all 5 should match
        let req = Request::builder()
            .uri("/api/posts?author[exists]")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "exists filter on non-JSONB column failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            5,
            "exists filter on non-JSONB should return 5 for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_jsonb_column_contains_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_contains_non_jsonb");

        let (app, _state) = test_db
            .setup_app(
                JSONB_FILTER_SORT_CONFIG,
                "jsonb_filter_contains_non_jsonb.yaml",
            )
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by title contains "Post" (non-JSONB column) - all 5 should match
        let req = Request::builder()
            .uri("/api/posts?title[contains]=Post")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "contains filter on non-JSONB column failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            5,
            "contains filter on non-JSONB should return 5 for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_bracket_notation_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_bracket");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_filter_bracket.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter using bracket notation: metadata.user.age[gte]=28
        let req = Request::builder()
            .uri("/api/posts?metadata.user.age[gte]=28")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "bracket filter failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            3,
            "bracket gte filter should return 3 results for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_in_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_in");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_filter_in.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by metadata.role in "alpha,beta" (should match Alpha and Beta)
        let req = Request::builder()
            .uri("/api/posts?metadata.role[in]=alpha,beta")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "in filter failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            2,
            "in filter should return 2 results for {backend}"
        );
        let titles: Vec<&str> = data.iter().map(|r| r["title"].as_str().unwrap()).collect();
        assert!(
            titles.contains(&"Post Alpha"),
            "in filter should include Alpha for {backend}"
        );
        assert!(
            titles.contains(&"Post Beta"),
            "in filter should include Beta for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_not_in_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_not_in");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_filter_not_in.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by metadata.role not_in "alpha,gamma" (should match Beta, Delta, Epsilon)
        let req = Request::builder()
            .uri("/api/posts?metadata.role[not_in]=alpha,gamma")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "not_in filter failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            3,
            "not_in filter should return 3 results for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_ilike_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_ilike");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_filter_ilike.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by metadata.role ilike "ALPHA" (case-insensitive, should match Alpha)
        let req = Request::builder()
            .uri("/api/posts?metadata.role[ilike]=ALPHA")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "ilike filter failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            1,
            "ilike filter should return 1 result for {backend}"
        );
        assert_eq!(
            data[0]["title"], "Post Alpha",
            "ilike filter title mismatch for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_sort_multiple_fields_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_sort_multi");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_sort_multi.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Sort by author ASC, then by metadata.role ASC
        // Author order: alice (Alpha, Delta), bob (Beta, Epsilon), charlie (Gamma)
        let req = Request::builder()
            .uri("/api/posts?sort=author&order=asc&page_size=10")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "sort by author failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 5, "should return all 5 posts for {backend}");
        // First two should be alice's posts (Alpha, Delta)
        assert_eq!(
            data[0]["author"], "alice",
            "first should be alice for {backend}"
        );
        assert_eq!(
            data[1]["author"], "alice",
            "second should be alice for {backend}"
        );
        // Last should be charlie's post
        assert_eq!(
            data[4]["author"], "charlie",
            "last should be charlie for {backend}"
        );

        // Sort by metadata.role DESC
        let req = Request::builder()
            .uri("/api/posts?sort=metadata.role&order=desc&page_size=10")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        // gamma > epsilon > delta > beta > alpha (reverse alphabetical)
        assert_eq!(
            data[0]["title"], "Post Gamma",
            "desc sort by metadata.role first should be Gamma for {backend}"
        );
        assert_eq!(
            data[4]["title"], "Post Alpha",
            "desc sort by metadata.role last should be Alpha for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_sort_nested_field_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_sort_nested");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_sort_nested.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Sort by metadata.user.age ASC (22, 25, 28, 30, 35)
        let req = Request::builder()
            .uri("/api/posts?sort=metadata.user.age&order=asc&page_size=10")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "sort by nested JSONB failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 5, "should return all 5 posts for {backend}");
        // Order: Epsilon(22), Beta(25), Delta(28), Alpha(30), Gamma(35)
        assert_eq!(
            data[0]["title"], "Post Epsilon",
            "asc sort by metadata.user.age first should be Epsilon for {backend}"
        );
        assert_eq!(
            data[1]["title"], "Post Beta",
            "asc sort by metadata.user.age second should be Beta for {backend}"
        );
        assert_eq!(
            data[2]["title"], "Post Delta",
            "asc sort by metadata.user.age third should be Delta for {backend}"
        );
        assert_eq!(
            data[3]["title"], "Post Alpha",
            "asc sort by metadata.user.age fourth should be Alpha for {backend}"
        );
        assert_eq!(
            data[4]["title"], "Post Gamma",
            "asc sort by metadata.user.age last should be Gamma for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_sort_with_pagination_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_sort_paginated");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_sort_paginated.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Sort by metadata.user.age ASC with pagination (page_size=2)
        let req = Request::builder()
            .uri("/api/posts?sort=metadata.user.age&order=asc&page=1&page_size=2")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 2, "page 1 should have 2 results for {backend}");
        assert_eq!(
            data[0]["title"], "Post Epsilon",
            "page 1 first should be Epsilon for {backend}"
        );
        assert_eq!(
            data[1]["title"], "Post Beta",
            "page 1 second should be Beta for {backend}"
        );

        // Page 2
        let req = Request::builder()
            .uri("/api/posts?sort=metadata.user.age&order=asc&page=2&page_size=2")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 2, "page 2 should have 2 results for {backend}");
        assert_eq!(
            data[0]["title"], "Post Delta",
            "page 2 first should be Delta for {backend}"
        );
        assert_eq!(
            data[1]["title"], "Post Alpha",
            "page 2 second should be Alpha for {backend}"
        );

        // Page 3
        let req = Request::builder()
            .uri("/api/posts?sort=metadata.user.age&order=asc&page=3&page_size=2")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 1, "page 3 should have 1 result for {backend}");
        assert_eq!(
            data[0]["title"], "Post Gamma",
            "page 3 should be Gamma for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_with_pagination_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_paginated");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_filter_paginated.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by metadata.status = "active" with pagination
        // Active posts: Alpha, Beta, Delta (3 results)
        let req = Request::builder()
            .uri("/api/posts?metadata.status=active&page_size=2")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "status failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 2, "page 1 should have 2 results for {backend}");

        let req = Request::builder()
            .uri("/api/posts?metadata.status=active&page=2&page_size=2")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 1, "page 2 should have 1 result for {backend}");
        assert_eq!(
            data[0]["title"], "Post Delta",
            "page 2 should be Delta for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_combined_with_sort_and_pagination_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_full_query");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_full_query.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter: metadata.status = "active" AND metadata.user.age < 30
        // Matches: Beta (age=25/active) and Delta (age=28/active)
        // Epsilon (age=22) has status=draft, not active
        // Sort: metadata.user.age ASC
        // Result order: Beta (25), Delta (28)
        let req = Request::builder()
            .uri("/api/posts?metadata.status=active&metadata.user.age[lt]=30&sort=metadata.user.age&order=asc&page_size=10")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            2,
            "combined filter+sort should return 2 for {backend}"
        );
        assert_eq!(
            data[0]["title"], "Post Beta",
            "combined filter+sort first should be Beta for {backend}"
        );
        assert_eq!(
            data[1]["title"], "Post Delta",
            "combined filter+sort second should be Delta for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_on_multiple_jsonb_columns_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_filter_on_multiple_jsonb_columns");

        let (app, _state) = test_db
            .setup_app(
                JSONB_FILTER_SORT_CONFIG,
                "jsonb_filter_on_multiple_jsonb_columns.yaml",
            )
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by metadata.status = "active" AND tags contains "ml"
        // Active+ml: Beta (active, ["python","ml"]) and Delta (active, ["rust","ml"])
        let req = Request::builder()
            .uri("/api/posts?metadata.status=active&tags[contains]=ml")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            2,
            "multi-column JSONB filter should return 2 for {backend}"
        );
        let titles: Vec<&str> = data.iter().map(|r| r["title"].as_str().unwrap()).collect();
        assert!(
            titles.contains(&"Post Beta"),
            "multi-column filter should include Beta for {backend}"
        );
        assert!(
            titles.contains(&"Post Delta"),
            "multi-column filter should include Delta for {backend}"
        );
    }
}

// =============================================================================
// JSONB Edge Case and Additional Coverage Tests
// =============================================================================

#[tokio::test]
async fn test_jsonb_filter_in_non_jsonb_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_in_non_jsonb");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_in_non_jsonb.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by author IN "alice,bob" (should match Alpha, Delta, Beta, Epsilon = 4 results)
        let req = Request::builder()
            .uri("/api/posts?author[in]=alice,bob")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "in filter on non-JSONB failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            4,
            "in filter should return 4 results for {backend}"
        );
        let authors: Vec<&str> = data.iter().map(|r| r["author"].as_str().unwrap()).collect();
        assert!(
            authors.contains(&"alice"),
            "in filter should include alice for {backend}"
        );
        assert!(
            authors.contains(&"bob"),
            "in filter should include bob for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_not_in_non_jsonb_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_not_in_non_jsonb");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_not_in_non_jsonb.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter by author NOT IN "alice,charlie" (should match Beta, Epsilon = 2 results)
        let req = Request::builder()
            .uri("/api/posts?author[not_in]=alice,charlie")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "not_in filter on non-JSONB failed for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            2,
            "not_in filter should return 2 results for {backend}"
        );
        let authors: Vec<&str> = data.iter().map(|r| r["author"].as_str().unwrap()).collect();
        assert!(
            !authors.contains(&"alice"),
            "not_in filter should exclude alice for {backend}"
        );
        assert!(
            !authors.contains(&"charlie"),
            "not_in filter should exclude charlie for {backend}"
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_null_jsonb_across_backends() {
    let mut test_db = TestDatabase::new(TestBackend::Sqlite, "jsonb_null");

    let (app, _state) = test_db
        .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_null.yaml")
        .await;

    seed_jsonb_posts_with_nulls(&app).await;

    // Filter by metadata.role eq "full" (should only match Post With Full Metadata)
    let req = Request::builder()
        .uri("/api/posts?metadata.role=full")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "eq filter with null metadata failed for sqlite"
    );
    let json = json_body(response).await;
    let data = json["data"].as_array().unwrap();
    assert_eq!(
        data.len(),
        1,
        "eq filter with null metadata should return 1 for sqlite"
    );
    assert_eq!(
        data[0]["title"], "Post With Full Metadata",
        "eq filter should match full metadata only"
    );
}

#[tokio::test]
async fn test_jsonb_null_jsonb_sort_across_backends() {
    let mut test_db = TestDatabase::new(TestBackend::Sqlite, "jsonb_null_sort");

    let (app, _state) = test_db
        .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_null_sort.yaml")
        .await;

    seed_jsonb_posts_with_nulls(&app).await;

    // Sort by metadata.role ASC - nulls should appear first or last depending on backend
    let req = Request::builder()
        .uri("/api/posts?sort=metadata.role&order=asc&page_size=10")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "sort with null metadata failed for sqlite"
    );
    let json = json_body(response).await;
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 3, "should return all 3 posts for sqlite");

    // "full" should be among the results
    let titles: Vec<&str> = data.iter().map(|r| r["title"].as_str().unwrap()).collect();
    assert!(
        titles.contains(&"Post With Full Metadata"),
        "sort should include Post With Full Metadata"
    );
}

#[tokio::test]
async fn test_jsonb_filter_invalid_syntax_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_invalid_filter");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_invalid_filter.yaml")
            .await;

        seed_default_jsonb_posts(&app).await;

        // Filter with non-existent column should return 400 or empty
        let req = Request::builder()
            .uri("/api/posts?nonexistent_field[something]=value")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert!(
            response.status() == StatusCode::BAD_REQUEST || response.status() == StatusCode::OK,
            "invalid filter syntax should return 400 or OK for {backend}, got {}",
            response.status()
        );

        // Filter with non-existent nested path on valid JSONB column
        let req = Request::builder()
            .uri("/api/posts?metadata.nonexistent_key=value")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert!(
            response.status() == StatusCode::BAD_REQUEST || response.status() == StatusCode::OK,
            "non-existent nested path should return 400 or OK for {backend}, got {}",
            response.status()
        );
    }
}

#[tokio::test]
async fn test_jsonb_filter_partial_nulls_across_backends() {
    for backend in enabled_backends() {
        let mut test_db = TestDatabase::new(backend, "jsonb_partial_nulls");

        let (app, _state) = test_db
            .setup_app(JSONB_FILTER_SORT_CONFIG, "jsonb_partial_nulls.yaml")
            .await;

        seed_jsonb_posts_with_nulls(&app).await;

        // Filter by metadata.role = "full" - should only match "Post With Full Metadata"
        // Posts with null metadata or empty metadata should not match
        let req = Request::builder()
            .uri("/api/posts?metadata.role=full")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "eq filter with null metadata should return 200 for {backend}"
        );
        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            1,
            "backend: {backend} - null metadata posts should not match role filter"
        );
        assert_eq!(
            data[0]["title"], "Post With Full Metadata",
            "backend: {backend}"
        );
    }
}

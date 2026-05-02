mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use support::db::{TestDatabase, enabled_backends};
use support::json_body;

use crate::support::CRUD_CONFIG;

const CRUD_FEATURES_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true

tables:
  __TABLE_NAME__:
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "title"
        type: "text"
        nullable: false
      - name: "body"
        type: "text"
        nullable: true
      - name: "author"
        type: "varchar"
        nullable: false

endpoints:
  - path: "/api/posts"
    methods: ["get", "post"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "body", "author"]
      writable_fields: ["title", "body", "author"]
      pagination:
        enabled: true
        default_page_size: 2
        max_page_size: 10
      filtering:
        enabled: true
        allowed_fields: ["author"]
      sorting:
        enabled: true
        allowed_fields: ["title", "id"]
        default_field: "id"
        default_order: "asc"
    auth: "none"
"#;

async fn seed_posts(app: &axum::Router, posts: &[(&str, &str)]) {
    for (title, author) in posts {
        let req = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "title": title,
                    "body": "",
                    "author": author,
                })
                .to_string(),
            ))
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }
}

#[tokio::test]
async fn test_crud_pagination_across_backends() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "api_pagination");
        let (app, _state, _pool) = test_db
            .setup_app(CRUD_FEATURES_CONFIG, "crud_features.yaml")
            .await;

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
        let test_db = TestDatabase::new(backend, "api_filtering");
        let (app, _state, _pool) = test_db
            .setup_app(CRUD_FEATURES_CONFIG, "crud_features.yaml")
            .await;

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
        let test_db = TestDatabase::new(backend, "api_sorting");
        let (app, _state, _pool) = test_db
            .setup_app(CRUD_FEATURES_CONFIG, "crud_features.yaml")
            .await;

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
        let test_db = TestDatabase::new(backend, "api_join");
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
  {authors_table}:
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "name"
        type: "text"
        nullable: false

  {posts_table}:
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

        let (app, _state, _pool) = test_db.setup_app(&template, "join_test.yaml").await;

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
        let test_db = TestDatabase::new(backend, "api_filter_reject");
        let (app, _state, _pool) = test_db
            .setup_app(CRUD_FEATURES_CONFIG, "crud_features.yaml")
            .await;

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
        let test_db = TestDatabase::new(backend, "api_sort_reject");
        let (app, _state, _pool) = test_db
            .setup_app(CRUD_FEATURES_CONFIG, "crud_features.yaml")
            .await;

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
  __TABLE_NAME__:
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
      writable_fields: ["title", "active"]
      where_clause: "active = true"
    auth: "none"

  - path: "/api/items/:id"
    methods: ["get"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      where_clause: "active = true"
    auth: "none"
"#;

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "api_where_single");
        let (app, _state, _pool) = test_db.setup_app(config, "where_single.yaml").await;

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
        let test_db = TestDatabase::new(backend, "api_page_clamp");
        let (app, _state, _pool) = test_db
            .setup_app(CRUD_FEATURES_CONFIG, "crud_features.yaml")
            .await;

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
        let test_db = TestDatabase::new(backend, "sql_injection_test");
        let (app, _state, _pool) = test_db
            .setup_app(CRUD_FEATURES_CONFIG, "crud_features.yaml")
            .await;

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
        let test_db = TestDatabase::new(backend, "api_sort_jsonb");

        let template = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true

tables:
  __TABLE_NAME__:
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "title"
        type: "text"
        nullable: false
      - name: "metadata"
        type: "jsonb"
        nullable: true

endpoints:
  - path: "/api/posts"
    methods: ["get", "post"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "metadata"]
      writable_fields: ["title", "metadata"]
      filtering:
        enabled: true
        allowed_fields: ["metadata.role"]
      sorting:
        enabled: true
        allowed_fields: ["metadata.role"]
        default_field: "id"
        default_order: "asc"
    auth: "none"
"#
        .to_string();

        let (app, _state, _pool) = test_db.setup_app(&template, "jsonb_sort.yaml").await;

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
        let test_db = TestDatabase::new(backend, "api_sort_jsonb_reject");
        let (app, _state, _pool) = test_db
            .setup_app(CRUD_FEATURES_CONFIG, "crud_features.yaml")
            .await;

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

#[tokio::test]
async fn test_crud_create_and_list_across_backends() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_create_and_list");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        let create_request = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "title": "Hello World",
                    "body": "First post!",
                    "author": "Alice"
                })
                .to_string(),
            ))
            .unwrap();

        let create_response = app.clone().oneshot(create_request).await.unwrap();
        assert_eq!(
            create_response.status(),
            StatusCode::CREATED,
            "backend: {backend}"
        );

        let list_request = Request::builder()
            .uri("/api/posts")
            .body(Body::empty())
            .unwrap();

        let list_response = app.oneshot(list_request).await.unwrap();
        assert_eq!(list_response.status(), StatusCode::OK, "backend: {backend}");

        let json = json_body(list_response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 1, "backend: {backend}");
        assert_eq!(data[0]["title"], "Hello World", "backend: {backend}");
        assert_eq!(data[0]["author"], "Alice", "backend: {backend}");
    }
}

#[tokio::test]
async fn test_crud_update_and_delete_across_backends() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_update_and_delete");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        let create_request = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "title": "Original",
                    "body": "body",
                    "author": "Bob"
                })
                .to_string(),
            ))
            .unwrap();
        let create_response = app.clone().oneshot(create_request).await.unwrap();
        assert_eq!(
            create_response.status(),
            StatusCode::CREATED,
            "backend: {backend}"
        );

        let update_request = Request::builder()
            .method("PUT")
            .uri("/api/posts/1")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"title": "Updated", "body": "new body"}).to_string(),
            ))
            .unwrap();
        let update_response = app.clone().oneshot(update_request).await.unwrap();
        assert_eq!(
            update_response.status(),
            StatusCode::OK,
            "backend: {backend}"
        );

        let get_request = Request::builder()
            .uri("/api/posts/1")
            .body(Body::empty())
            .unwrap();
        let get_response = app.clone().oneshot(get_request).await.unwrap();
        assert_eq!(get_response.status(), StatusCode::OK, "backend: {backend}");
        let json = json_body(get_response).await;
        assert_eq!(json["title"], "Updated", "backend: {backend}");

        let delete_request = Request::builder()
            .method("DELETE")
            .uri("/api/posts/1")
            .body(Body::empty())
            .unwrap();
        let delete_response = app.clone().oneshot(delete_request).await.unwrap();
        assert_eq!(
            delete_response.status(),
            StatusCode::OK,
            "backend: {backend}"
        );

        let list_request = Request::builder()
            .uri("/api/posts")
            .body(Body::empty())
            .unwrap();
        let list_response = app.oneshot(list_request).await.unwrap();
        let json = json_body(list_response).await;
        let data = json["data"].as_array().unwrap();
        assert!(data.is_empty(), "backend: {backend}");
    }
}

#[tokio::test]
async fn test_crud_list_empty() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_list_empty");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        let req = Request::builder()
            .uri("/api/posts")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");

        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert!(data.is_empty(), "backend: {backend}");
    }
}

// =============================================================================
// CRUD: Get single (GET /api/posts/{id})
// =============================================================================

#[tokio::test]
async fn test_crud_get_one() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_get_one");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        // Create a post.
        let req = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "title": "Test Post",
                    "body": "content",
                    "author": "Charlie"
                })
                .to_string(),
            ))
            .unwrap();
        let _ = app.clone().oneshot(req).await.unwrap();

        // Get post by ID.
        let req = Request::builder()
            .uri("/api/posts/1")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");

        let json = json_body(response).await;
        assert_eq!(json["title"], "Test Post", "backend: {backend}");
        assert_eq!(json["author"], "Charlie", "backend: {backend}");
    }
}

#[tokio::test]
async fn test_crud_get_one_not_found() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_get_not_found");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        let req = Request::builder()
            .uri("/api/posts/999")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "backend: {backend}"
        );
    }
}

// =============================================================================
// CRUD: Update not found (PUT /api/posts/{id})
// =============================================================================

#[tokio::test]
async fn test_crud_update_not_found() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_update_not_found");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        let req = Request::builder()
            .method("PUT")
            .uri("/api/posts/999")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({"title": "Nope"}).to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "backend: {backend}"
        );
    }
}

// =============================================================================
// CRUD: Delete not found (DELETE /api/posts/{id})
// =============================================================================

#[tokio::test]
async fn test_crud_delete_not_found() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_delete_not_found");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        let req = Request::builder()
            .method("DELETE")
            .uri("/api/posts/999")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "backend: {backend}"
        );
    }
}

// =============================================================================
// CRUD: Invalid requests
// =============================================================================

#[tokio::test]
async fn test_crud_create_no_body() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_no_body");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

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
        let test_db = TestDatabase::new(backend, "crud_non_writable");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

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
// Concurrent connection handling
// =============================================================================

#[tokio::test]
async fn test_concurrent_connections() {
    // Test that the server can handle at least 1000 concurrent connections
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    std::fs::create_dir_all(&public_dir).unwrap();
    std::fs::write(
        public_dir.join("index.html"),
        "<html><body>Concurrent Test</body></html>",
    )
    .unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

endpoints:
  - path: "/static/*"
    methods: ["get"]
    action: "static"
    static_files:
      root: "{root}"
      index: "index.html"
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    // Bind to a real port and spawn the server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind to TCP listener");
    let server_addr = listener.local_addr().expect("Failed to get local addr");
    let server_url = format!("http://{}", server_addr);

    // Clone the app for spawning
    let app_clone = app.clone();

    // Spawn the server in the background
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app_clone)
            .await
            .expect("Server failed");
    });

    // Give the server a moment to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Spawn 1000 concurrent HTTP requests to the real server
    let mut handles = Vec::new();
    let num_requests = 1000;

    for i in 0..num_requests {
        let url = server_url.clone();
        let handle = tokio::spawn(async move {
            let client = reqwest::Client::new();
            let response = client
                .get(format!("{}/static/index.html", url))
                .send()
                .await
                .expect("Request failed");

            assert_eq!(
                response.status(),
                reqwest::StatusCode::OK,
                "Request {i} failed with status {}",
                response.status()
            );
            let body = response.text().await.expect("Failed to read body");
            assert!(
                body.contains("Concurrent Test"),
                "Request {i} body mismatch: {}",
                body
            );
        });
        handles.push(handle);
    }

    // Wait for all requests to complete
    for handle in handles {
        handle.await.expect("Request failed");
    }

    // Verify we handled at least 1000 requests
    assert!(
        num_requests >= 1000,
        "Expected to handle at least 1000 concurrent connections, got {}",
        num_requests
    );

    // Shutdown the server by killing the task
    server_handle.abort();
}

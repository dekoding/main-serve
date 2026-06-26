//! Headless blog backend scenario.
//!
//! Tests Main Serve configured as a headless CMS / blog backend with:
//!
//! - User registration and login (password-based auth)
//! - JWT token authentication
//! - CRUD endpoints for posts (public read, authenticated write)
//! - Pagination, sorting, and filtering
//! - Role-based access control (admin, author, user)
//! - Soft-delete via status filtering (published vs draft)
//!
//! Config: based on `config/templates/blog.yaml` with port 0 and SQLite.

use crate::support::{BinaryHandle, LiveClient};
use serde_json::json;
use reqwest::StatusCode;

const BLOG_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

logging:
  level: "info"
  format: "json"

databases:
  main:
    driver: "sqlite"
    url: "sqlite://blog.db?mode=rwc"
    auto_migrate: true

tables:
  - name: "users"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
        nullable: false
      - name: "email"
        type: "varchar"
        unique: true
        nullable: false
        indexed: true
      - name: "password_hash"
        type: "text"
        nullable: false
      - name: "role"
        type: "varchar"
        nullable: false
        default: "'user'"
      - name: "created_at"
        type: "timestamptz"
        nullable: false
        default: "CURRENT_TIMESTAMP"
      - name: "updated_at"
        type: "timestamptz"
        nullable: false
        default: "CURRENT_TIMESTAMP"

  - name: "posts"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
        nullable: false
      - name: "title"
        type: "varchar"
        nullable: false
      - name: "slug"
        type: "varchar"
        unique: true
        nullable: false
        indexed: true
      - name: "body"
        type: "text"
        nullable: false
      - name: "excerpt"
        type: "text"
        nullable: true
      - name: "author_id"
        type: "bigint"
        nullable: false
        indexed: true
      - name: "status"
        type: "varchar"
        nullable: false
        default: "'draft'"
      - name: "published_at"
        type: "timestamptz"
        nullable: true
      - name: "created_at"
        type: "timestamptz"
        nullable: false
        default: "CURRENT_TIMESTAMP"
      - name: "updated_at"
        type: "timestamptz"
        nullable: false
        default: "CURRENT_TIMESTAMP"

auth:
  jwt:
    secret: "blog-test-secret-key"
    algorithm: "HS256"
    issuer: "main-serve-blog"
    expiry: 3600
    role_claim: "role"

  register:
    enabled: true
    table: "users"
    database: "main"
    default_role: "user"
    password_hash: "argon2id"

endpoints:
  - path: "/api/users"
    methods: ["get", "post"]
    action: "crud"
    auth: "jwt"
    roles: ["admin"]
    crud:
      table: "users"
      database: "main"
      fields: ["id", "email", "role", "created_at"]
      writable_fields: ["email", "password_hash", "role"]
      pagination:
        enabled: true
        default_page_size: 20
        max_page_size: 100
      filtering:
        enabled: true
        allowed_fields: ["email", "role", "created_at"]

  - path: "/api/users/{id}"
    methods: ["get"]
    action: "crud"
    auth: "jwt"
    roles: ["admin"]
    crud:
      table: "users"
      database: "main"
      fields: ["id", "email", "role", "created_at"]

  - path: "/api/posts"
    methods: ["get"]
    action: "crud"
    auth: "none"
    crud:
      table: "posts"
      database: "main"
      fields: ["id", "title", "slug", "author_id", "status", "published_at", "created_at", "excerpt"]
      pagination:
        enabled: true
        default_page_size: 10
        max_page_size: 50
      filtering:
        enabled: true
        allowed_fields: ["status", "author_id", "created_at"]
      sorting:
        enabled: true
        default_field: "published_at"
        default_order: "desc"
        allowed_fields: ["id", "title", "published_at", "created_at"]
      where_clause: "status = 'published'"

  - path: "/api/posts"
    methods: ["post"]
    action: "crud"
    auth: "jwt"
    roles: ["admin", "author"]
    crud:
      table: "posts"
      database: "main"
      fields: ["id", "title", "slug", "author_id", "status", "published_at", "created_at", "excerpt"]
      writable_fields: ["title", "slug", "body", "excerpt", "status", "published_at"]
      insert_owner: "author_id"

  - path: "/api/posts/{id}"
    methods: ["get", "put", "delete"]
    action: "crud"
    auth: "jwt"
    roles: ["admin", "author"]
    crud:
      table: "posts"
      database: "main"
      fields: ["id", "title", "slug", "body", "author_id", "status", "published_at", "created_at", "updated_at", "excerpt"]
      writable_fields: ["title", "slug", "body", "excerpt", "status", "published_at"]
      update_where_clause: "author_id = ${request.user.id}"
      delete_where_clause: "author_id = ${request.user.id}"

  - path: "/api/info"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"name": "Main Serve Blog", "version": "0.2.2"}'
"#;

#[tokio::test]
async fn test_health_endpoint() {
    let server = BinaryHandle::spawn(BLOG_CONFIG, None).await.expect("server spawn");
    let client = server.client();

    let resp = client
        .get_json::<serde_json::Value>("/api/info")
        .await
        .expect("GET /api/info");

    assert_eq!(resp.get("name").and_then(|v| v.as_str()), Some("Main Serve Blog"));
    assert_eq!(
        resp.get("version").and_then(|v| v.as_str()),
        Some("0.2.2")
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_user_registration() {
    let server = BinaryHandle::spawn(BLOG_CONFIG, None).await.expect("server spawn");
    let client = server.client();

    // Register a new user
    let resp = client
        .post_json("/register", &json!({
            "email": "alice@example.com",
            "password": "testpassword123"
        }))
        .await
        .expect("POST /register");

    let _ = client.assert_status(&resp, StatusCode::CREATED).await;

    let body = resp.json::<serde_json::Value>().await.expect("parse response");
    let user_id = body.get("id").or_else(|| body.get("data").and_then(|d| d.get("id")))
        .and_then(|v| v.as_i64())
        .expect("user id in response");

    assert!(user_id > 0);

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_auth_flow() {
    let server = BinaryHandle::spawn(BLOG_CONFIG, None).await.expect("server spawn");
    let client = server.client();

    // Register a user first
    client
        .post_json("/register", &json!({
            "email": "alice@example.com",
            "password": "testpassword123"
        }))
        .await
        .expect("register")
        .error_for_status()
        .ok()
        .expect("register succeeded");

    // Login
    let resp = client
        .post_json("/auth/login", &json!({
            "email": "alice@example.com",
            "password": "testpassword123"
        }))
        .await
        .expect("POST /auth/login");

    let _ = client.assert_status(&resp, StatusCode::OK).await;

    let body = resp.json::<serde_json::Value>().await.expect("parse login response");
    let token = body.get("token")
        .and_then(|v| v.as_str())
        .expect("token in login response");

    assert!(!token.is_empty());

    // Use token to access protected endpoint
    let authed_client = LiveClient::new(server.base_url())
        .with_bearer_token(token);

    let resp = authed_client
        .get_json::<serde_json::Value>("/api/posts")
        .await
        .expect("GET /api/posts with token");

    // Should return an empty list (no published posts yet)
    let results = resp.get("results").or_else(|| resp.get("data"))
        .expect("results in response");
    assert_eq!(results.as_array().map(|a| a.len()), Some(0));

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_create_and_list_posts() {
    let server = BinaryHandle::spawn(BLOG_CONFIG, None).await.expect("server spawn");
    let client = server.client();

    // Register and login as author
    client
        .post_json("/register", &json!({
            "email": "author@example.com",
            "password": "authorpass123"
        }))
        .await
        .expect("register")
        .error_for_status()
        .ok();

    let login_resp = client
        .post_json("/auth/login", &json!({
            "email": "author@example.com",
            "password": "authorpass123"
        }))
        .await
        .expect("login");

    let _ = client.assert_status(&login_resp, StatusCode::OK).await;
    let login_body = login_resp.json::<serde_json::Value>().await.expect("parse login");
    let token = login_body.get("token").and_then(|v| v.as_str()).expect("token");

    let authed = LiveClient::new(server.base_url())
        .with_bearer_token(token);

    // Create a draft post (not published, so not visible in public feed)
    authed.post_json("/api/posts", &json!({
        "title": "My Draft Post",
        "slug": "my-draft-post",
        "body": "This is a draft post body.",
        "status": "draft"
    }))
    .await
    .expect("create draft")
    .error_for_status()
    .ok();

    // Public feed should be empty (only published posts)
    let public_client = LiveClient::new(server.base_url());
    let feed = public_client
        .get_json::<serde_json::Value>("/api/posts")
        .await
        .expect("get feed");

    let results = feed.get("results").or_else(|| feed.get("data"))
        .and_then(|v| v.as_array())
        .expect("results array");
    assert_eq!(results.len(), 0, "draft post should not appear in public feed");

    // Create a published post
    authed.post_json("/api/posts", &json!({
        "title": "My Published Post",
        "slug": "my-published-post",
        "body": "This is a published post body.",
        "status": "published",
        "published_at": "2025-01-15T12:00:00Z"
    }))
    .await
    .expect("create published")
    .error_for_status()
    .ok();

    // Now the public feed should have one post
    let feed = public_client
        .get_json::<serde_json::Value>("/api/posts")
        .await
        .expect("get feed");

    let results = feed.get("results").or_else(|| feed.get("data"))
        .and_then(|v| v.as_array())
        .expect("results array");
    assert_eq!(results.len(), 1);

    let first = &results[0];
    assert_eq!(first.get("title").and_then(|v| v.as_str()), Some("My Published Post"));
    assert_eq!(first.get("status").and_then(|v| v.as_str()), Some("published"));

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_pagination() {
    let server = BinaryHandle::spawn(BLOG_CONFIG, None).await.expect("server spawn");
    let client = server.client();

    // Register and login
    client
        .post_json("/register", &json!({
            "email": "editor@example.com",
            "password": "editorpass123"
        }))
        .await
        .expect("register")
        .error_for_status()
        .ok();

    let login_resp = client
        .post_json("/auth/login", &json!({
            "email": "editor@example.com",
            "password": "editorpass123"
        }))
        .await
        .expect("login");

    let _ = client.assert_status(&login_resp, StatusCode::OK).await;
    let login_body = login_resp.json::<serde_json::Value>().await.expect("parse login");
    let token = login_body.get("token").and_then(|v| v.as_str()).expect("token");

    let authed = LiveClient::new(server.base_url())
        .with_bearer_token(token);

    // Create 5 published posts
    for i in 1..=5 {
        authed.post_json("/api/posts", &json!({
            "title": &format!("Post {}", i),
            "slug": &format!("post-{}", i),
            "body": format!("Body of post {}", i),
            "status": "published",
            "published_at": "2025-01-15T12:00:00Z"
        }))
        .await
        .expect("create post")
        .error_for_status()
        .ok();
    }

    // Get page 1, page size 2
    let feed = client
        .get_json::<serde_json::Value>("/api/posts?page=1&page_size=2")
        .await
        .expect("get page 1");

    let results = feed.get("results").or_else(|| feed.get("data"))
        .and_then(|v| v.as_array())
        .expect("results");
    assert_eq!(results.len(), 2);

    // Page 2
    let feed = client
        .get_json::<serde_json::Value>("/api/posts?page=2&page_size=2")
        .await
        .expect("get page 2");

    let results = feed.get("results").or_else(|| feed.get("data"))
        .and_then(|v| v.as_array())
        .expect("results");
    assert_eq!(results.len(), 2);

    // Verify total count metadata
    let total = feed.get("total").or_else(|| feed.get("data").and_then(|d| d.get("total")))
        .and_then(|v| v.as_u64())
        .expect("total in response");
    assert_eq!(total, 5);

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_filtering_and_sorting() {
    let server = BinaryHandle::spawn(BLOG_CONFIG, None).await.expect("server spawn");
    let client = server.client();

    // Register and login
    client
        .post_json("/register", &json!({
            "email": "admin@example.com",
            "password": "adminpass123"
        }))
        .await
        .expect("register")
        .error_for_status()
        .ok();

    let login_resp = client
        .post_json("/auth/login", &json!({
            "email": "admin@example.com",
            "password": "adminpass123"
        }))
        .await
        .expect("login");

    let _ = client.assert_status(&login_resp, StatusCode::OK).await;
    let login_body = login_resp.json::<serde_json::Value>().await.expect("parse login");
    let token = login_body.get("token").and_then(|v| v.as_str()).expect("token");

    let authed = LiveClient::new(server.base_url())
        .with_bearer_token(token);

    // Create posts with different dates
    authed.post_json("/api/posts", &json!({
        "title": "Old Post",
        "slug": "old-post",
        "body": "old",
        "status": "published",
        "published_at": "2024-01-01T00:00:00Z"
    }))
    .await
    .expect("create old")
    .error_for_status()
    .ok();

    authed.post_json("/api/posts", &json!({
        "title": "New Post",
        "slug": "new-post",
        "body": "new",
        "status": "published",
        "published_at": "2025-06-15T00:00:00Z"
    }))
    .await
    .expect("create new")
    .error_for_status()
    .ok();

    // Default sort is by published_at desc (newest first)
    let feed = client
        .get_json::<serde_json::Value>("/api/posts?sort=published_at&order=desc")
        .await
        .expect("sorted feed");

    let results = feed.get("results").or_else(|| feed.get("data"))
        .and_then(|v| v.as_array())
        .expect("results");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].get("title").and_then(|v| v.as_str()), Some("New Post"));
    assert_eq!(results[1].get("title").and_then(|v| v.as_str()), Some("Old Post"));

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_unauthorized_access() {
    let server = BinaryHandle::spawn(BLOG_CONFIG, None).await.expect("server spawn");
    let client = server.client();

    // Try to access admin endpoint without auth
    let resp = client
        .get("/api/users")
        .await
        .expect("GET /api/users");

    let _ = client.assert_status(&resp, StatusCode::UNAUTHORIZED).await;

    server.shutdown().await.expect("server shutdown");
}

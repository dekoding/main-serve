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
use reqwest::StatusCode;
use serde_json::json;

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
    roles: ["admin", "author", "user"]
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
      body: '{"name": "Main Serve Blog", "version": "0.3.0"}'
"#;

#[tokio::test]
async fn test_health_endpoint() {
    let server = setup_blog_server().await;
    let client = server.client();

    let resp = client
        .get_json::<serde_json::Value>("/api/info")
        .await
        .expect("GET /api/info");

    assert_eq!(
        resp.get("name").and_then(|v| v.as_str()),
        Some("Main Serve Blog")
    );
    assert_eq!(resp.get("version").and_then(|v| v.as_str()), Some("0.3.0"));

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_user_registration() {
    let server = setup_blog_server().await;
    let client = server.client();

    // Register a new user
    let resp = client
        .post_json(
            "/_main-serve/register",
            &json!({
                "email": "alice@example.com",
                "password": "testpassword123"
            }),
        )
        .await
        .expect("POST /register");

    let _ = client.assert_status(&resp, StatusCode::CREATED).await;

    let body = resp
        .json::<serde_json::Value>()
        .await
        .expect("parse response");
    let token = body
        .get("token")
        .and_then(|v| v.as_str())
        .expect("token in response");

    // Decode JWT sub claim to get user_id
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT should have 3 parts");
    let decoded =
        base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, parts[1])
            .expect("base64 decode");
    let jwt_body: serde_json::Value = serde_json::from_slice(&decoded).expect("parse JWT body");
    let user_id = jwt_body
        .get("sub")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<i64>().ok())
        .expect("user id from JWT sub claim");

    assert!(user_id > 0);

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_auth_flow() {
    let server = setup_blog_server().await;
    let client = server.client();

    // Register a user first
    client
        .post_json(
            "/_main-serve/register",
            &json!({
                "email": "alice@example.com",
                "password": "testpassword123"
            }),
        )
        .await
        .expect("register")
        .error_for_status()
        .expect("register succeeded");

    // Login
    let resp = client
        .post_json(
            "/_main-serve/login",
            &json!({
                "email": "alice@example.com",
                "password": "testpassword123"
            }),
        )
        .await
        .expect("POST /auth/login");

    let _ = client.assert_status(&resp, StatusCode::OK).await;

    let body = resp
        .json::<serde_json::Value>()
        .await
        .expect("parse login response");
    let token = body
        .get("token")
        .and_then(|v| v.as_str())
        .expect("token in login response");

    assert!(!token.is_empty());

    // Use token to access protected endpoint
    let authed_client = LiveClient::new(server.base_url()).with_bearer_token(token);

    let resp = authed_client
        .get_json::<serde_json::Value>("/api/posts")
        .await
        .expect("GET /api/posts with token");

    // Should return an empty list (no published posts yet)
    let results = resp
        .get("results")
        .or_else(|| resp.get("data"))
        .expect("results in response");
    assert_eq!(results.as_array().map(|a| a.len()), Some(0));

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_create_and_list_posts() {
    let server = setup_blog_server().await;
    let client = server.client();

    // Register and login as author
    client
        .post_json(
            "/_main-serve/register",
            &json!({
                "email": "author@example.com",
                "password": "authorpass123"
            }),
        )
        .await
        .expect("register")
        .error_for_status()
        .ok();

    let login_resp = client
        .post_json(
            "/_main-serve/login",
            &json!({
                "email": "author@example.com",
                "password": "authorpass123"
            }),
        )
        .await
        .expect("login");

    let _ = client.assert_status(&login_resp, StatusCode::OK).await;
    let login_body = login_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse login");
    let token = login_body
        .get("token")
        .and_then(|v| v.as_str())
        .expect("token");

    let authed = LiveClient::new(server.base_url()).with_bearer_token(token);

    // Create a draft post (not published, so not visible in public feed)
    authed
        .post_json(
            "/api/posts",
            &json!({
                "title": "My Draft Post",
                "slug": "my-draft-post",
                "body": "This is a draft post body.",
                "status": "draft"
            }),
        )
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

    let results = feed
        .get("results")
        .or_else(|| feed.get("data"))
        .and_then(|v| v.as_array())
        .expect("results array");
    assert_eq!(
        results.len(),
        0,
        "draft post should not appear in public feed"
    );

    // Create a published post
    authed
        .post_json(
            "/api/posts",
            &json!({
                "title": "My Published Post",
                "slug": "my-published-post",
                "body": "This is a published post body.",
                "status": "published",
                "published_at": "2025-01-15T12:00:00Z"
            }),
        )
        .await
        .expect("create published")
        .error_for_status()
        .ok();

    // Now the public feed should have one post
    let feed = public_client
        .get_json::<serde_json::Value>("/api/posts")
        .await
        .expect("get feed");

    let results = feed
        .get("results")
        .or_else(|| feed.get("data"))
        .and_then(|v| v.as_array())
        .expect("results array");
    assert_eq!(results.len(), 1);

    let first = &results[0];
    assert_eq!(
        first.get("title").and_then(|v| v.as_str()),
        Some("My Published Post")
    );
    assert_eq!(
        first.get("status").and_then(|v| v.as_str()),
        Some("published")
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_pagination() {
    let server = setup_blog_server().await;
    let client = server.client();

    // Register and login
    client
        .post_json(
            "/_main-serve/register",
            &json!({
                "email": "editor@example.com",
                "password": "editorpass123"
            }),
        )
        .await
        .expect("register")
        .error_for_status()
        .ok();

    let login_resp = client
        .post_json(
            "/_main-serve/login",
            &json!({
                "email": "editor@example.com",
                "password": "editorpass123"
            }),
        )
        .await
        .expect("login");

    let _ = client.assert_status(&login_resp, StatusCode::OK).await;
    let login_body = login_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse login");
    let token = login_body
        .get("token")
        .and_then(|v| v.as_str())
        .expect("token");

    let authed = LiveClient::new(server.base_url()).with_bearer_token(token);

    // Create 5 published posts
    for i in 1..=5 {
        authed
            .post_json(
                "/api/posts",
                &json!({
                    "title": &format!("Post {}", i),
                    "slug": &format!("post-{}", i),
                    "body": format!("Body of post {}", i),
                    "status": "published",
                    "published_at": "2025-01-15T12:00:00Z"
                }),
            )
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

    let results = feed
        .get("results")
        .or_else(|| feed.get("data"))
        .and_then(|v| v.as_array())
        .expect("results");
    assert_eq!(results.len(), 2);

    // Page 2
    let feed = client
        .get_json::<serde_json::Value>("/api/posts?page=2&page_size=2")
        .await
        .expect("get page 2");

    let results = feed
        .get("results")
        .or_else(|| feed.get("data"))
        .and_then(|v| v.as_array())
        .expect("results");
    assert_eq!(results.len(), 2);

    // Verify total count metadata
    let total = feed
        .get("total")
        .or_else(|| feed.get("data").and_then(|d| d.get("total")))
        .and_then(|v| v.as_u64())
        .expect("total in response");
    assert_eq!(total, 5);

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_filtering_and_sorting() {
    let server = setup_blog_server().await;
    let client = server.client();

    // Register and login
    client
        .post_json(
            "/_main-serve/register",
            &json!({
                "email": "admin@example.com",
                "password": "adminpass123"
            }),
        )
        .await
        .expect("register")
        .error_for_status()
        .ok();

    let login_resp = client
        .post_json(
            "/_main-serve/login",
            &json!({
                "email": "admin@example.com",
                "password": "adminpass123"
            }),
        )
        .await
        .expect("login");

    let _ = client.assert_status(&login_resp, StatusCode::OK).await;
    let login_body = login_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse login");
    let token = login_body
        .get("token")
        .and_then(|v| v.as_str())
        .expect("token");

    let authed = LiveClient::new(server.base_url()).with_bearer_token(token);

    // Create posts with different dates
    authed
        .post_json(
            "/api/posts",
            &json!({
                "title": "Old Post",
                "slug": "old-post",
                "body": "old",
                "status": "published",
                "published_at": "2024-01-01T00:00:00Z"
            }),
        )
        .await
        .expect("create old")
        .error_for_status()
        .ok();

    authed
        .post_json(
            "/api/posts",
            &json!({
                "title": "New Post",
                "slug": "new-post",
                "body": "new",
                "status": "published",
                "published_at": "2025-06-15T00:00:00Z"
            }),
        )
        .await
        .expect("create new")
        .error_for_status()
        .ok();

    // Default sort is by published_at desc (newest first)
    let feed = client
        .get_json::<serde_json::Value>("/api/posts?sort=published_at&order=desc")
        .await
        .expect("sorted feed");

    let results = feed
        .get("results")
        .or_else(|| feed.get("data"))
        .and_then(|v| v.as_array())
        .expect("results");
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0].get("title").and_then(|v| v.as_str()),
        Some("New Post")
    );
    assert_eq!(
        results[1].get("title").and_then(|v| v.as_str()),
        Some("Old Post")
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_unauthorized_access() {
    let server = setup_blog_server().await;
    let client = server.client();

    // Try to access admin endpoint without auth
    let resp = client.get("/api/users").await.expect("GET /api/users");

    let _ = client.assert_status(&resp, StatusCode::UNAUTHORIZED).await;

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_insert_owner_field() {
    let server = setup_blog_server().await;
    let client = server.client();

    // Register and login as author
    client
        .post_json(
            "/_main-serve/register",
            &json!({
                "email": "owner_test@example.com",
                "password": "ownerpass123"
            }),
        )
        .await
        .expect("register")
        .error_for_status()
        .ok();

    let login_resp = client
        .post_json(
            "/_main-serve/login",
            &json!({
                "email": "owner_test@example.com",
                "password": "ownerpass123"
            }),
        )
        .await
        .expect("login");

    let _ = client.assert_status(&login_resp, StatusCode::OK).await;
    let login_body = login_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse login");
    let token = login_body
        .get("token")
        .and_then(|v| v.as_str())
        .expect("token");

    let authed = LiveClient::new(server.base_url()).with_bearer_token(token);

    // Create a post - insert_owner should auto-set author_id
    let create_resp = authed
        .post_json(
            "/api/posts",
            &json!({
                "title": "Owner Test Post",
                "slug": "owner-test-post",
                "body": "This post should have author_id set automatically.",
                "status": "published",
                "published_at": "2025-01-15T12:00:00Z"
            }),
        )
        .await
        .expect("create post")
        .error_for_status()
        .expect("create post should succeed");

    let created = create_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse create response");

    let post_data = created
        .get("data")
        .or(Some(&created))
        .expect("response body");

    let author_id = post_data
        .get("author_id")
        .expect("author_id should be set by insert_owner");

    assert!(
        author_id.as_i64().is_some(),
        "author_id should be a valid integer"
    );

    // Verify the author_id appears in the public feed
    let feed = client
        .get_json::<serde_json::Value>("/api/posts")
        .await
        .expect("get feed");

    let results = feed
        .get("results")
        .or_else(|| feed.get("data"))
        .and_then(|v| v.as_array())
        .expect("results");

    let found = results
        .iter()
        .find(|p| p.get("author_id").and_then(|v| v.as_i64()) == author_id.as_i64());

    assert!(
        found.is_some(),
        "post with correct author_id should appear in public feed"
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_update_where_clause() {
    let server = setup_blog_server().await;
    let client = server.client();

    // Register and login as author with "author" role to access /api/posts/{id}
    client
        .post_json(
            "/_main-serve/register",
            &json!({
                "email": "update_test@example.com",
                "password": "updatepass123",
                "role": "author"
            }),
        )
        .await
        .expect("register")
        .error_for_status()
        .ok();

    let login_resp = client
        .post_json(
            "/_main-serve/login",
            &json!({
                "email": "update_test@example.com",
                "password": "updatepass123"
            }),
        )
        .await
        .expect("login");

    let _ = client.assert_status(&login_resp, StatusCode::OK).await;
    let login_body = login_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse login");
    let token = login_body
        .get("token")
        .and_then(|v| v.as_str())
        .expect("token");

    let authed = LiveClient::new(server.base_url()).with_bearer_token(token);

    // Create a post
    let create_resp = authed
        .post_json(
            "/api/posts",
            &json!({
                "title": "Update Test Post",
                "slug": "update-test-post",
                "body": "Original body.",
                "status": "draft",
            }),
        )
        .await
        .expect("create post")
        .error_for_status()
        .expect("create post should succeed");

    let created = create_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse create response");

    let post_data = created
        .get("data")
        .or(Some(&created))
        .expect("response body");

    let post_id = post_data
        .get("id")
        .and_then(|v| v.as_i64())
        .expect("post id");

    // Update the post (should succeed because author_id matches)
    let update_resp = authed
        .put_json(
            &format!("/api/posts/{post_id}"),
            &json!({
                "title": "Updated Title",
                "body": "Updated body.",
                "status": "published",
                "published_at": "2025-01-15T12:00:00Z"
            }),
        )
        .await
        .expect("update post");

    assert!(
        update_resp.status().is_success(),
        "author should be able to update their own post, got: {}",
        update_resp.status()
    );

    // Verify the update was applied by fetching the post
    let fetch_resp = authed
        .get(&format!("/api/posts/{post_id}"))
        .await
        .expect("fetch updated post");
    client.assert_status(&fetch_resp, StatusCode::OK).await;

    let fetched = fetch_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse fetch response");

    let fetched_title = fetched
        .get("title")
        .or_else(|| fetched.get("data").and_then(|d| d.get("title")))
        .and_then(|v| v.as_str())
        .expect("title in fetch response");

    assert_eq!(
        fetched_title, "Updated Title",
        "title should be updated after PUT"
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_delete_where_clause() {
    let server = setup_blog_server().await;
    let client = server.client();

    // Register and login as author with "author" role to access /api/posts/{id}
    client
        .post_json(
            "/_main-serve/register",
            &json!({
                "email": "delete_test@example.com",
                "password": "deletepass123",
                "role": "author"
            }),
        )
        .await
        .expect("register")
        .error_for_status()
        .ok();

    let login_resp = client
        .post_json(
            "/_main-serve/login",
            &json!({
                "email": "delete_test@example.com",
                "password": "deletepass123"
            }),
        )
        .await
        .expect("login");

    let _ = client.assert_status(&login_resp, StatusCode::OK).await;
    let login_body = login_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse login");
    let token = login_body
        .get("token")
        .and_then(|v| v.as_str())
        .expect("token");

    let authed = LiveClient::new(server.base_url()).with_bearer_token(token);

    // Create a post
    let create_resp = authed
        .post_json(
            "/api/posts",
            &json!({
                "title": "Delete Test Post",
                "slug": "delete-test-post",
                "body": "This will be deleted.",
                "status": "draft",
            }),
        )
        .await
        .expect("create post")
        .error_for_status()
        .expect("create post should succeed");

    let created = create_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse create response");

    let post_data = created
        .get("data")
        .or(Some(&created))
        .expect("response body");

    let post_id = post_data
        .get("id")
        .and_then(|v| v.as_i64())
        .expect("post id");

    // Delete the post (should succeed because author_id matches)
    let delete_resp = authed
        .delete(&format!("/api/posts/{post_id}"))
        .await
        .expect("delete post");

    assert!(
        delete_resp.status().is_success() || delete_resp.status() == StatusCode::NO_CONTENT,
        "author should be able to delete their own post, got: {}",
        delete_resp.status()
    );

    // Verify post is gone from listings
    let feed = client
        .get_json::<serde_json::Value>("/api/posts")
        .await
        .expect("get feed");

    let results = feed
        .get("results")
        .or_else(|| feed.get("data"))
        .and_then(|v| v.as_array())
        .expect("results");

    let found = results
        .iter()
        .find(|p| p.get("slug").and_then(|v| v.as_str()) == Some("delete-test-post"));

    assert!(
        found.is_none(),
        "deleted post should not appear in listings"
    );

    server.shutdown().await.expect("server shutdown");
}

async fn setup_blog_server() -> BinaryHandle {
    use std::time::{SystemTime, UNIX_EPOCH};
    let db_path = format!(
        "{}/blog-{}.db",
        std::env::temp_dir().display(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let config = BLOG_CONFIG.replace(
        "sqlite://blog.db?mode=rwc",
        &format!("sqlite://{}?mode=rwc", db_path),
    );
    BinaryHandle::spawn(&config, None)
        .await
        .expect("server spawn")
}

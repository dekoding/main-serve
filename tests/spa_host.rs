/// Integration tests for the SPA host endpoint handler.
///
/// Tests SPA fallback behavior, config parsing, store references,
/// method restrictions, and route registration.
mod support;

use axum::body::Body;
use axum::http::Request;
use main_serve::config::types::{EndpointAction, StoreBackend};
use support::helpers::load_yaml;
use tower::ServiceExt;

fn root_str(dir: &tempfile::TempDir) -> String {
    dir.path().display().to_string()
}

// =============================================================================
// SPA host config parsing
// =============================================================================

#[test]
fn test_spa_host_config_parses_basic() {
    let yaml = r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "./public"

endpoints:
  - path: "/app/*"
    methods: ["get"]
    action: "spa_host"
    spa_host:
      storage: "assets"
      index: "index.html"
      cache_max_age: 86400
      etag: true
      head_support: true
      fallback_status: 200
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    assert_eq!(config.stores.len(), 1);
    let store = config.stores.get("assets").unwrap();
    assert_eq!(store.backend, StoreBackend::Native);

    let endpoint = config.endpoints.first().unwrap();
    assert!(matches!(endpoint.action, EndpointAction::SpaHost));
    let spa_config = endpoint.spa_host.as_ref().unwrap();
    assert_eq!(spa_config.storage, "assets");
    assert_eq!(spa_config.index, "index.html");
    assert_eq!(spa_config.cache_max_age, 86400);
    assert!(spa_config.etag);
    assert!(spa_config.head_support);
    assert_eq!(spa_config.fallback_status, 200);
}

#[test]
fn test_spa_host_config_parses_with_cache_rules() {
    let yaml = r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "./public"

endpoints:
  - path: "/app/*"
    methods: ["get"]
    action: "spa_host"
    spa_host:
      storage: "assets"
      index: "index.html"
      cache_max_age: 3600
      cache_rules:
        - extensions: [".js", ".css"]
          cache_control: "public, max-age=604800"
        - extensions: [".html"]
          cache_control: "no-cache"
      etag: true
      head_support: true
      fallback_status: 200
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let endpoint = config.endpoints.first().unwrap();
    let spa_config = endpoint.spa_host.as_ref().unwrap();
    assert_eq!(spa_config.cache_rules.len(), 2);
}

#[test]
fn test_spa_host_config_default_values() {
    let yaml = r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "./public"

endpoints:
  - path: "/app/*"
    methods: ["get"]
    action: "spa_host"
    spa_host:
      storage: "assets"
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let endpoint = config.endpoints.first().unwrap();
    let spa_config = endpoint.spa_host.as_ref().unwrap();
    assert_eq!(spa_config.index, "index.html");
    assert_eq!(spa_config.cache_max_age, 3600);
    assert!(spa_config.etag);
    assert!(spa_config.head_support);
    assert_eq!(spa_config.fallback_status, 200);
}

#[test]
fn test_spa_host_config_with_store_ref() {
    let yaml = r#"
server:
  port: 0

stores:
  frontend:
    backend: native
    root: "/var/www/app"

endpoints:
  - path: "/*"
    methods: ["get", "head"]
    action: "spa_host"
    spa_host:
      storage: "frontend"
      index: "index.html"
      cache_max_age: 0
      etag: false
      head_support: true
      fallback_status: 200
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let endpoint = config.endpoints.first().unwrap();
    let spa_config = endpoint.spa_host.as_ref().unwrap();
    assert_eq!(spa_config.storage, "frontend");
    assert_eq!(spa_config.index, "index.html");
    assert!(!spa_config.etag);
}

// =============================================================================
// SPA host validation errors
// =============================================================================

#[test]
fn test_spa_host_missing_storage_field() {
    let yaml = r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "./public"

endpoints:
  - path: "/app/*"
    methods: ["get"]
    action: "spa_host"
    spa_host: {}
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("storage") || err.contains("must not be empty"),
        "Error should mention storage: {err}"
    );
}

#[test]
fn test_spa_host_empty_storage_field() {
    let yaml = r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "./public"

endpoints:
  - path: "/app/*"
    methods: ["get"]
    action: "spa_host"
    spa_host:
      storage: ""
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("storage must not be empty"), "{err}");
}

#[test]
fn test_spa_host_invalid_store_ref() {
    let yaml = r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "./public"

endpoints:
  - path: "/app/*"
    methods: ["get"]
    action: "spa_host"
    spa_host:
      storage: "nonexistent"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("nonexistent"),
        "Error should mention missing store: {err}"
    );
}

#[test]
fn test_spa_host_missing_index_file_fallback() {
    let yaml = r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "./public"

endpoints:
  - path: "/app/*"
    methods: ["get"]
    action: "spa_host"
    spa_host:
      storage: "assets"
      index: "app.html"
      fallback_status: 200
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let endpoint = config.endpoints.first().unwrap();
    let spa_config = endpoint.spa_host.as_ref().unwrap();
    assert_eq!(spa_config.index, "app.html");
    assert_eq!(spa_config.fallback_status, 200);
}

// =============================================================================
// SPA host handler routing tests (integration)
//
// Note: Routes use wildcard patterns like "/app/*" which become "/app/{*rest}".
// These only match paths WITH content after the prefix (e.g., "/app/foo", "/app/index.html").
// Bare paths like "/app" or "/app/" do NOT match the wildcard route.
// =============================================================================

#[tokio::test]
async fn test_spa_host_get_missing_file_returns_404() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let yaml = format!(
        r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/app/*"
    methods: ["get"]
    action: "spa_host"
    spa_host:
      storage: "assets"
      index: "index.html"
    auth: "none"
"#,
        root = root_str(&dir),
    );
    let (app, _f) = support::setup_server(&yaml).await;

    // Use /app/index.html to match the wildcard route
    let req = Request::builder()
        .uri("/app/index.html")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    // File doesn't exist in empty temp dir - returns 404 (with SPA fallback attempting index.html which also doesn't exist)
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_spa_host_deep_path_returns_404() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let yaml = format!(
        r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/app/*"
    methods: ["get"]
    action: "spa_host"
    spa_host:
      storage: "assets"
      index: "index.html"
    auth: "none"
"#,
        root = root_str(&dir),
    );
    let (app, _f) = support::setup_server(&yaml).await;

    let req = Request::builder()
        .uri("/app/deep/nested/path/file.js")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_spa_host_post_rejected() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let yaml = format!(
        r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/app/*"
    methods: ["get"]
    action: "spa_host"
    spa_host:
      storage: "assets"
      index: "index.html"
    auth: "none"
"#,
        root = root_str(&dir),
    );
    let (app, _f) = support::setup_server(&yaml).await;

    // POST to a path that matches the wildcard route
    let req = Request::builder()
        .method("POST")
        .uri("/app/index.html")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    // POST should be rejected since spa_host only allows GET (and HEAD)
    assert_eq!(
        response.status(),
        axum::http::StatusCode::METHOD_NOT_ALLOWED
    );
}

#[tokio::test]
async fn test_spa_head_returns_404_for_missing_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let yaml = format!(
        r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/app/*"
    methods: ["get", "head"]
    action: "spa_host"
    spa_host:
      storage: "assets"
      index: "index.html"
      head_support: true
    auth: "none"
"#,
        root = root_str(&dir),
    );
    let (app, _f) = support::setup_server(&yaml).await;

    let req = Request::builder()
        .method("HEAD")
        .uri("/app/index.html")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    // HEAD for non-existent index.html returns 404
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_spa_host_wildcard_covers_nested_paths() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let yaml = format!(
        r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/app/*"
    methods: ["get"]
    action: "spa_host"
    spa_host:
      storage: "assets"
      index: "index.html"
    auth: "none"
"#,
        root = root_str(&dir),
    );
    let (app, _f) = support::setup_server(&yaml).await;

    // Nested paths with content should all match the wildcard
    let paths = [
        "/app/index.html",
        "/app/js/app.js",
        "/app/css/style.css",
        "/app/a/b/c/d.html",
    ];

    for path in paths {
        let req = Request::builder().uri(path).body(Body::empty()).unwrap();

        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::NOT_FOUND,
            "Path {path} should match spa_host endpoint but file doesn't exist"
        );
    }
}

#[tokio::test]
async fn test_spa_host_non_wildcard_path_not_matched() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let yaml = format!(
        r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/app/*"
    methods: ["get"]
    action: "spa_host"
    spa_host:
      storage: "assets"
      index: "index.html"
    auth: "none"
"#,
        root = root_str(&dir),
    );
    let (app, _f) = support::setup_server(&yaml).await;

    // Path without /app prefix should not match
    let req = Request::builder()
        .uri("/other/index.html")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_spa_host_with_multiple_endpoints() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let yaml = format!(
        r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/health"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      body: "{{\"status\":\"ok\"}}"
      content_type: "application/json"
    auth: "none"

  - path: "/app/*"
    methods: ["get"]
    action: "spa_host"
    spa_host:
      storage: "assets"
      index: "index.html"
    auth: "none"
"#,
        root = root_str(&dir),
    );
    let (app, _f) = support::setup_server(&yaml).await;

    // Health endpoint should work
    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    // SPA endpoint returns 404 for missing file in empty store
    let req = Request::builder()
        .uri("/app/index.html")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

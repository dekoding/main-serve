// Middleware integration tests.
//
// These tests verify:
// - CORS headers are applied correctly
// - Request IDs are generated and returned
// - Compression is negotiated
// - Rate limiting blocks excess requests

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use main_serve::server::{AppState, build_router};

const CORS_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

cors:
  allowed_origins:
    - "https://example.com"
  allowed_methods:
    - "GET"
    - "POST"
  allowed_headers:
    - "Content-Type"
    - "Authorization"
  allow_credentials: true
  max_age: 3600

endpoints:
  - path: "/hello"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"message": "hello"}'
    auth: "none"
"#;

const RATE_LIMIT_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

rate_limit:
  enabled: true
  max_requests: 3
  window_seconds: 60
  key_strategy: header
  key_header: "X-Client-Id"

endpoints:
  - path: "/limited"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"ok": true}'
    auth: "none"
"#;

const MINIMAL_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

endpoints:
  - path: "/ping"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "text/plain"
      body: "pong"
    auth: "none"
"#;

// =========================================================================
// CORS tests
// =========================================================================

#[tokio::test]
async fn test_cors_allows_configured_origin() {
    let (app, _f) = support::setup_server(CORS_CONFIG).await;

    let req = Request::builder()
        .method("OPTIONS")
        .uri("/hello")
        .header("Origin", "https://example.com")
        .header("Access-Control-Request-Method", "GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .map(|v| v.to_str().unwrap()),
        Some("https://example.com")
    );
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-credentials")
            .map(|v| v.to_str().unwrap()),
        Some("true")
    );
}

#[tokio::test]
async fn test_cors_rejects_unconfigured_origin() {
    let (app, _f) = support::setup_server(CORS_CONFIG).await;

    let req = Request::builder()
        .method("OPTIONS")
        .uri("/hello")
        .header("Origin", "https://evil.com")
        .header("Access-Control-Request-Method", "GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();

    // CORS layer should not include the allow-origin header for a disallowed origin.
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
}

#[tokio::test]
async fn test_cors_max_age() {
    let (app, _f) = support::setup_server(CORS_CONFIG).await;

    let req = Request::builder()
        .method("OPTIONS")
        .uri("/hello")
        .header("Origin", "https://example.com")
        .header("Access-Control-Request-Method", "GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();

    assert_eq!(
        response
            .headers()
            .get("access-control-max-age")
            .map(|v| v.to_str().unwrap()),
        Some("3600")
    );
}

// =========================================================================
// Request ID tests
// =========================================================================

#[tokio::test]
async fn test_request_id_generated() {
    let (app, _f) = support::setup_server(MINIMAL_CONFIG).await;

    let req = Request::builder()
        .uri("/_main-serve/health")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Should have an x-request-id header with a UUID.
    let request_id = response.headers().get("x-request-id");
    assert!(
        request_id.is_some(),
        "x-request-id header should be present"
    );
    let id_str = request_id.unwrap().to_str().unwrap();
    assert!(
        uuid::Uuid::parse_str(id_str).is_ok(),
        "x-request-id should be a valid UUID"
    );
}

#[tokio::test]
async fn test_request_id_propagated() {
    let (app, _f) = support::setup_server(MINIMAL_CONFIG).await;

    let custom_id = "my-custom-request-id-123";
    let req = Request::builder()
        .uri("/_main-serve/health")
        .header("x-request-id", custom_id)
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Should propagate the client-provided request ID.
    let returned_id = response
        .headers()
        .get("x-request-id")
        .map(|v| v.to_str().unwrap());
    assert_eq!(returned_id, Some(custom_id));
}

// =========================================================================
// Rate limiting tests
// =========================================================================

#[tokio::test]
async fn test_rate_limit_blocks_excess_requests() {
    let (app, _f) = support::setup_server(RATE_LIMIT_CONFIG).await;

    // First 3 requests should succeed.
    for i in 0..3 {
        let req = Request::builder()
            .uri("/limited")
            .header("X-Client-Id", "client-1")
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "Request {i} should succeed"
        );
    }

    // 4th request should be rate limited.
    let req = Request::builder()
        .uri("/limited")
        .header("X-Client-Id", "client-1")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn test_rate_limit_different_clients_independent() {
    let (app, _f) = support::setup_server(RATE_LIMIT_CONFIG).await;

    // Exhaust client-a's limit.
    for _ in 0..3 {
        let req = Request::builder()
            .uri("/limited")
            .header("X-Client-Id", "client-a")
            .body(Body::empty())
            .unwrap();
        app.clone().oneshot(req).await.unwrap();
    }

    // client-b should still be allowed.
    let req = Request::builder()
        .uri("/limited")
        .header("X-Client-Id", "client-b")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// =========================================================================
// Compression tests
// =========================================================================

#[tokio::test]
async fn test_compression_negotiated() {
    let (app, _f) = support::setup_server(MINIMAL_CONFIG).await;

    let req = Request::builder()
        .uri("/_main-serve/health")
        .header("Accept-Encoding", "gzip")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // When Accept-Encoding: gzip is sent, the compression layer should set
    // content-encoding to indicate the response is compressed.
    let content_encoding = response
        .headers()
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        content_encoding, "gzip",
        "Response should be gzip-encoded when Accept-Encoding: gzip is sent"
    );
}

#[tokio::test]
async fn test_no_compression_without_accept_encoding() {
    let (app, _f) = support::setup_server(MINIMAL_CONFIG).await;

    let req = Request::builder()
        .uri("/_main-serve/health")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    // Without Accept-Encoding, there should be no content-encoding header.
    assert!(response.headers().get("content-encoding").is_none());
}

// =========================================================================
// Per-endpoint CORS override
// =========================================================================

#[tokio::test]
async fn test_per_endpoint_cors_override() {
    let yaml = r#"
server:
  port: 0

cors:
  allowed_origins:
    - "https://global.example.com"
  allowed_methods:
    - "GET"

endpoints:
  - path: "/global"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: "global"
    auth: "none"

  - path: "/custom"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: "custom"
    cors:
      allowed_origins:
        - "https://special.example.com"
      allowed_methods:
        - "GET"
        - "POST"
      max_age: 600
    auth: "none"
"#;
    let (app, _f) = support::setup_server(yaml).await;

    // Global endpoint should accept the global origin.
    let req = Request::builder()
        .method("OPTIONS")
        .uri("/global")
        .header("origin", "https://global.example.com")
        .header("access-control-request-method", "GET")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let origin = response
        .headers()
        .get("access-control-allow-origin")
        .map(|v| v.to_str().unwrap().to_string());
    assert_eq!(origin.as_deref(), Some("https://global.example.com"));

    // Custom endpoint should accept the special origin.
    let req = Request::builder()
        .method("OPTIONS")
        .uri("/custom")
        .header("origin", "https://special.example.com")
        .header("access-control-request-method", "GET")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let origin = response
        .headers()
        .get("access-control-allow-origin")
        .map(|v| v.to_str().unwrap().to_string());
    assert_eq!(origin.as_deref(), Some("https://special.example.com"));

    // Custom endpoint should have its own max-age.
    let max_age = response
        .headers()
        .get("access-control-max-age")
        .map(|v| v.to_str().unwrap().to_string());
    assert_eq!(max_age.as_deref(), Some("600"));
}

// =========================================================================
// Max body size enforcement
// =========================================================================

#[tokio::test]
async fn test_max_body_size_enforced() {
    // DefaultBodyLimit only applies when the handler extracts the body.
    // CRUD POST handlers use Json<Value> extraction, which triggers the limit.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let db_path = dir.path().join("test.db");

    let yaml = format!(
        r#"
server:
  port: 0
  max_body_size: 64

databases:
  main:
    driver: sqlite
    url: "sqlite://{db_path}?mode=rwc"

tables:
  items:
    database: main
    columns:
      - name: id
        type: serial
        primary_key: true
      - name: title
        type: text

endpoints:
  - path: "/api/items"
    methods: ["post"]
    action: crud
    crud:
      table: items
      database: main
    auth: none
"#,
        db_path = db_path.display()
    );

    let config_file = dir.path().join("config.yaml");
    std::fs::write(&config_file, &yaml).unwrap();
    let config = main_serve::config::load_config(&config_file).expect("load config");
    let pools = main_serve::db::pool::create_pools(&config.databases)
        .await
        .expect("create pools");
    main_serve::db::migration::run_migrations(&config.tables, &pools, &config.databases)
        .await
        .expect("migrations");
    let state = AppState::new(config, config_file, "test-token".to_string());
    {
        let mut pool_lock = state.db_pools.write().await;
        *pool_lock = pools;
    }
    let config_guard = state.config.read().await;
    let app = build_router(&config_guard, state.clone());
    drop(config_guard);

    // Small body should succeed.
    let req = Request::builder()
        .method("POST")
        .uri("/api/items")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"title":"hi"}"#))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Body exceeding max_body_size (64 bytes) should be rejected.
    let large_title = "x".repeat(100);
    let large_body = format!(r#"{{"title":"{large_title}"}}"#);
    let req = Request::builder()
        .method("POST")
        .uri("/api/items")
        .header("content-type", "application/json")
        .body(Body::from(large_body))
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

// =========================================================================
// Body logging
// =========================================================================

/// When log_request_body is enabled, the request body must still reach the
/// handler intact (the middleware buffers and replays it).
#[tokio::test]
async fn test_body_logging_preserves_request_body() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let db_path = dir.path().join("test.db");

    let yaml = format!(
        r#"
server:
  port: 0

logging:
  log_request_body: true
  log_response_body: false

databases:
  main:
    driver: sqlite
    url: "sqlite://{db_path}?mode=rwc"

tables:
  items:
    database: main
    columns:
      - name: id
        type: serial
        primary_key: true
      - name: title
        type: text

endpoints:
  - path: "/api/items"
    methods: ["post", "get"]
    action: crud
    crud:
      table: items
      database: main
    auth: none
"#,
        db_path = db_path.display()
    );

    let config_file = dir.path().join("config.yaml");
    std::fs::write(&config_file, &yaml).unwrap();
    let config = main_serve::config::load_config(&config_file).expect("load config");
    assert!(config.logging.log_request_body);

    let pools = main_serve::db::pool::create_pools(&config.databases)
        .await
        .expect("create pools");
    main_serve::db::migration::run_migrations(&config.tables, &pools, &config.databases)
        .await
        .expect("migrations");
    let state = AppState::new(config, config_file, "test-token".to_string());
    {
        let mut pool_lock = state.db_pools.write().await;
        *pool_lock = pools;
    }
    let config_guard = state.config.read().await;
    let app = build_router(&config_guard, state.clone());
    drop(config_guard);

    // POST a JSON body - body logging should buffer it, but the CRUD
    // handler should still receive the data and create the record.
    let req = Request::builder()
        .method("POST")
        .uri("/api/items")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"title":"logged"}"#))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Verify the record was actually created (body wasn't consumed).
    let req = Request::builder()
        .method("GET")
        .uri("/api/items")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = json["data"].as_array().expect("items array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "logged");
}

/// When log_response_body is enabled, the response body must still be
/// returned to the client intact.
#[tokio::test]
async fn test_body_logging_preserves_response_body() {
    let yaml = r#"
server:
  port: 0

logging:
  log_request_body: false
  log_response_body: true

endpoints:
  - path: "/info"
    methods: ["get"]
    action: custom_response
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"msg":"hello from body logging test"}'
    auth: none
"#;
    let (app, _f) = support::setup_server(yaml).await;

    let req = Request::builder()
        .method("GET")
        .uri("/info")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // The response body must be intact even though it was buffered for logging.
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["msg"], "hello from body logging test");
}

/// When both flags are disabled, the middleware layer should not be added.
/// Verify normal operation is unaffected.
#[tokio::test]
async fn test_body_logging_disabled_no_effect() {
    let yaml = r#"
server:
  port: 0

logging:
  log_request_body: false
  log_response_body: false

endpoints:
  - path: "/ping"
    methods: ["get"]
    action: custom_response
    custom_response:
      status: 200
      body: "pong"
    auth: none
"#;
    let (app, _f) = support::setup_server(yaml).await;

    let req = Request::builder()
        .method("GET")
        .uri("/ping")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"pong");
}

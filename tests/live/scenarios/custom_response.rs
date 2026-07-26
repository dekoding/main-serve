//! Custom response scenario.
//!
//! Tests Main Serve configured with custom_response endpoints:
//!
//! - Static JSON responses with custom status codes
//! - Custom content-type headers
//! - Custom response headers
//! - Body content verification
//!
//! Config: based on the custom_response action from config/templates/example.yaml.

use crate::support::BinaryHandle;
use reqwest::StatusCode;

const CUSTOM_RESPONSE_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

logging:
  level: "info"
  format: "json"

endpoints:
  - path: "/api/health"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"status": "ok", "name": "main-serve"}'
      headers:
        X-Health-Check: "true"
    auth: "none"

  - path: "/api/info"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"name": "Main Serve", "version": "0.1.1"}'
      headers:
        X-Custom-A: "hello"
        X-Custom-B: "world"
    auth: "none"

  - path: "/redirect"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 302
      content_type: "text/html"
      body: "Redirecting..."
      headers:
        Location: "https://example.com"
    auth: "none"

  - path: "/not-found"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 404
      content_type: "application/json"
      body: '{"error": "not found", "code": 404}'
    auth: "none"

  - path: "/forbidden"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 403
      content_type: "application/json"
      body: '{"error": "forbidden", "code": 403}'
    auth: "none"
"#;

#[tokio::test]
async fn test_custom_health_endpoint() {
    let server = setup_custom_server().await;
    let client = server.client();

    let resp = client
        .get_json::<serde_json::Value>("/api/health")
        .await
        .expect("GET /api/health");

    assert_eq!(resp.get("status").and_then(|v| v.as_str()), Some("ok"));
    assert_eq!(
        resp.get("name").and_then(|v| v.as_str()),
        Some("main-serve")
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_custom_response_raw_body() {
    let server = setup_custom_server().await;
    let client = server.client();

    let resp = client.get("/api/health").await.expect("GET /api/health");
    let body = resp.text().await.expect("read body");
    assert_eq!(body, r#"{"status": "ok", "name": "main-serve"}"#);

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_custom_info_endpoint() {
    let server = setup_custom_server().await;
    let client = server.client();

    let resp = client
        .get_json::<serde_json::Value>("/api/info")
        .await
        .expect("GET /api/info");

    assert_eq!(
        resp.get("name").and_then(|v| v.as_str()),
        Some("Main Serve")
    );
    assert_eq!(resp.get("version").and_then(|v| v.as_str()), Some("0.1.1"));

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_custom_response_headers() {
    let server = setup_custom_server().await;
    let client = server.client();

    let resp = client.get("/api/health").await.expect("GET /api/health");

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok());
    assert!(
        content_type.is_some_and(|ct| ct.contains("application/json")),
        "content-type should be application/json"
    );

    let x_health = resp
        .headers()
        .get("x-health-check")
        .and_then(|v| v.to_str().ok());
    assert!(
        x_health.is_some_and(|v| v == "true"),
        "X-Health-Check header should be 'true'"
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_custom_redirect() {
    let server = setup_custom_server().await;
    let client = server.client();

    let resp = client.get("/redirect").await.expect("GET /redirect");

    assert_eq!(resp.status(), StatusCode::FOUND, "should return 302 Found");

    let location = resp.headers().get("location").and_then(|v| v.to_str().ok());
    assert!(
        location.is_some_and(|l| l == "https://example.com"),
        "Location header should point to example.com"
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_custom_not_found() {
    let server = setup_custom_server().await;
    let client = server.client();

    let resp = client
        .get_json::<serde_json::Value>("/not-found")
        .await
        .expect("GET /not-found");

    assert_eq!(
        resp.get("error").and_then(|v| v.as_str()),
        Some("not found")
    );
    assert_eq!(resp.get("code").and_then(|v| v.as_u64()), Some(404));

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_custom_forbidden() {
    let server = setup_custom_server().await;
    let client = server.client();

    let resp = client
        .get_json::<serde_json::Value>("/forbidden")
        .await
        .expect("GET /forbidden");

    assert_eq!(
        resp.get("error").and_then(|v| v.as_str()),
        Some("forbidden")
    );

    server.shutdown().await.expect("server shutdown");
}

async fn setup_custom_server() -> BinaryHandle {
    BinaryHandle::spawn(CUSTOM_RESPONSE_CONFIG, None)
        .await
        .expect("spawn server")
}

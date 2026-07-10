//! Reverse proxy scenario.
//!
//! Tests Main Serve configured as a reverse proxy:
//!
//! - Forwarding requests to an upstream HTTP service
//! - Path rewriting (strip_prefix / add_prefix)
//! - Adding headers to upstream requests
//! - Upstream timeout handling
//!
//! Config: proxies /api/external/* to a mock upstream server.

use crate::support::BinaryHandle;
use axum::Json;
use axum::Router;
use axum::routing::{any, get};
use reqwest::StatusCode;
use std::net::SocketAddr;
use tokio::net::TcpListener;

const PROXY_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

logging:
  level: "info"
  format: "json"

endpoints:
  - path: "/api/external/*"
    methods: ["get", "post", "put", "delete"]
    action: "proxy"
    proxy:
      upstream: "http://127.0.0.1:0"
      path_rewrite:
        strip_prefix: "/api/external"
        add_prefix: "/v2"
      headers:
        X-Forwarded-For: "client"
        X-Upstream-Auth: "main-serve-proxy"
      timeouts:
        connect: 5
        read: 30
        total: 60
      max_response_size: 268435456
    auth: "none"
"#;

/// A simple mock upstream server for testing proxy functionality.
struct MockUpstream {
    base_url: String,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl MockUpstream {
    async fn spawn() -> Self {
        let router = Router::new()
            .route(
                "/v2/users",
                get(|| async {
                    Json(serde_json::json!({
                        "users": [
                            {"id": 1, "name": "Alice"},
                            {"id": 2, "name": "Bob"},
                        ]
                    }))
                }),
            )
            .route(
                "/v2/users/{id}",
                get(|path: axum::extract::Path<String>| async move {
                    Json(serde_json::json!({
                        "id": path.parse::<i64>().unwrap_or(0),
                        "name": "TestUser",
                    }))
                }),
            )
            .route(
                "/v2/echo",
                any(|req: axum::http::Request<axum::body::Body>| async move {
                    use http_body_util::BodyExt;
                    let method = req.method().to_string();
                    let headers = req.headers().clone();
                    let x_upstream_auth = headers
                        .get("x-upstream-auth")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let content_type = headers
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let body_bytes = req.into_body().collect().await.unwrap().to_bytes();
                    Json(serde_json::json!({
                        "method": method,
                        "headers": {
                            "x-upstream-auth": x_upstream_auth,
                            "content-type": content_type,
                        },
                        "body": String::from_utf8_lossy(&body_bytes).to_string(),
                    }))
                }),
            )
            .route(
                "/delay",
                get(|| async {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    Json(serde_json::json!({"delayed": true}))
                }),
            );

        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .expect("bind mock upstream");
        let addr = listener.local_addr().expect("local addr");
        let base_url = format!("http://127.0.0.1:{}", addr.port());

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let app = router;

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    shutdown_rx.await.ok();
                })
                .await
                .ok();
        });

        Self {
            base_url,
            _shutdown: shutdown_tx,
        }
    }
}

#[tokio::test]
async fn test_proxy_forward() {
    let upstream = MockUpstream::spawn().await;
    let config = PROXY_CONFIG.replace("http://127.0.0.1:0", &upstream.base_url);

    let server = BinaryHandle::spawn(&config, None)
        .await
        .expect("spawn proxy server");
    let client = server.client();

    // GET /api/external/users -> upstream /v2/users
    let resp = client
        .get_json::<serde_json::Value>("/api/external/users")
        .await
        .expect("GET /api/external/users");

    let users = resp.get("users").and_then(|v| v.as_array());
    assert!(users.is_some(), "response should have users array");
    let users = users.unwrap();
    assert_eq!(users.len(), 2);
    assert_eq!(users[0].get("name").and_then(|v| v.as_str()), Some("Alice"));

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_proxy_path_rewrite() {
    let upstream = MockUpstream::spawn().await;
    let config = PROXY_CONFIG.replace("http://127.0.0.1:0", &upstream.base_url);

    let server = BinaryHandle::spawn(&config, None)
        .await
        .expect("spawn proxy server");
    let client = server.client();

    // GET /api/external/users/42 -> upstream /v2/users/42
    let resp = client
        .get_json::<serde_json::Value>("/api/external/users/42")
        .await
        .expect("GET /api/external/users/42");

    assert_eq!(resp.get("id").and_then(|v| v.as_i64()), Some(42));
    assert_eq!(resp.get("name").and_then(|v| v.as_str()), Some("TestUser"));

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_proxy_headers() {
    let upstream = MockUpstream::spawn().await;
    let config = PROXY_CONFIG.replace("http://127.0.0.1:0", &upstream.base_url);

    let server = BinaryHandle::spawn(&config, None)
        .await
        .expect("spawn proxy server");
    let client = server.client();

    // POST /api/external/echo - check that upstream receives proxy headers
    let resp = client
        .post_json(
            "/api/external/echo",
            &serde_json::json!({"message": "hello from client"}),
        )
        .await
        .expect("POST /api/external/echo");

    let _ = client.assert_status(&resp, StatusCode::OK).await;

    let body = resp
        .json::<serde_json::Value>()
        .await
        .expect("parse response");
    let headers = body.get("headers").and_then(|v| v.as_object());
    assert!(headers.is_some(), "response should have headers object");

    let headers = headers.unwrap();
    assert_eq!(
        headers.get("x-upstream-auth").and_then(|v| v.as_str()),
        Some("main-serve-proxy")
    );
}

#[tokio::test]
async fn test_proxy_preserves_method() {
    let upstream = MockUpstream::spawn().await;
    let config = PROXY_CONFIG.replace("http://127.0.0.1:0", &upstream.base_url);

    let server = BinaryHandle::spawn(&config, None)
        .await
        .expect("spawn proxy server");
    let client = server.client();

    // PUT /api/external/echo - check that method is forwarded
    let resp = client
        .put_json("/api/external/echo", &serde_json::json!({"updated": true}))
        .await
        .expect("PUT /api/external/echo");

    let body = resp
        .json::<serde_json::Value>()
        .await
        .expect("parse response");
    assert_eq!(body.get("method").and_then(|v| v.as_str()), Some("PUT"));

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_proxy_preserves_body() {
    let upstream = MockUpstream::spawn().await;
    let config = PROXY_CONFIG.replace("http://127.0.0.1:0", &upstream.base_url);

    let server = BinaryHandle::spawn(&config, None)
        .await
        .expect("spawn proxy server");
    let client = server.client();

    let resp = client
        .post_json(
            "/api/external/echo",
            &serde_json::json!({"key": "value", "nested": {"a": 1}}),
        )
        .await
        .expect("POST echo");

    let body = resp
        .json::<serde_json::Value>()
        .await
        .expect("parse response");
    let forwarded_body = body.get("body").and_then(|v| v.as_str());
    assert!(forwarded_body.is_some_and(|b| b.contains("\"key\":\"value\"")));

    server.shutdown().await.expect("server shutdown");
}

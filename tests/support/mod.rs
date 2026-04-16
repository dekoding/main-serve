#![allow(dead_code)]

pub mod db;

use std::io::Write;

use axum::body::Body;
use axum::http::Request;
use axum::response::IntoResponse;
use http_body_util::BodyExt;
use tempfile::NamedTempFile;

use main_serve::config::load_config;
use main_serve::server::{AppState, build_router};

/// Set up a server from a YAML string without database pools.
///
/// Returns the router and the temp file handle. The caller must keep the
/// `NamedTempFile` alive for the duration of the test to prevent the
/// underlying config file from being deleted (required for reload tests).
pub async fn setup_server(yaml: &str) -> (axum::Router, NamedTempFile) {
    let mut f = NamedTempFile::new().expect("tempfile");
    f.write_all(yaml.as_bytes()).expect("write");
    let config = load_config(f.path()).expect("load config");
    let state = AppState::new(config, f.path().to_path_buf(), "test-token".to_string());
    let config_guard = state.config.read().await;
    let app = build_router(&config_guard, state.clone());
    drop(config_guard);
    (app, f)
}

/// Parse a JSON response body.
pub async fn json_body(response: axum::http::Response<Body>) -> serde_json::Value {
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

/// Standard CRUD config template used by multiple test files.
///
/// Contains `__DB_DRIVER__`, `__DB_URL__`, and `__TABLE_NAME__` placeholders
/// that `TestDatabase::setup_app()` replaces automatically.
pub const CRUD_CONFIG: &str = r#"
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
    auth: "none"

  - path: "/api/posts/{id}"
    methods: ["get", "put", "delete"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "body", "author"]
      writable_fields: ["title", "body", "author"]
    auth: "none"
"#;

/// Start a mock OAuth2 identity provider that handles /token and /userinfo.
///
/// The mock returns different user profiles based on the Bearer token:
///   - "admin-token-*" -> sub: "admin-42", role: "admin"
///   - "revoked-*"     -> 401 Unauthorized (simulates expired/revoked token)
///   - anything else   -> sub: "user-99",  role: "user"
///
/// The /token endpoint always succeeds and returns "mock-access-token".
///
/// Returns (base_url, shutdown_sender).
pub async fn start_mock_idp() -> (String, tokio::sync::oneshot::Sender<()>) {
    use axum::Router;
    use axum::routing::{get, post};
    use std::net::SocketAddr;
    use tokio::net::TcpListener;

    let app = Router::new()
        .route(
            "/token",
            post(|| async {
                axum::Json(serde_json::json!({
                    "access_token": "mock-access-token",
                    "token_type": "Bearer",
                    "expires_in": 3600,
                }))
            }),
        )
        .route(
            "/userinfo",
            get(|req: Request<Body>| async move {
                let auth = req
                    .headers()
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                let token = auth.strip_prefix("Bearer ").unwrap_or("");

                if token.starts_with("revoked-") {
                    return axum::http::Response::builder()
                        .status(axum::http::StatusCode::UNAUTHORIZED)
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(
                            serde_json::json!({"error": "invalid_token"}).to_string(),
                        ))
                        .unwrap()
                        .into_response();
                }

                if token.starts_with("admin-token") {
                    axum::Json(serde_json::json!({
                        "sub": "admin-42",
                        "role": "admin",
                        "email": "admin@example.com",
                    }))
                    .into_response()
                } else {
                    axum::Json(serde_json::json!({
                        "sub": "user-99",
                        "role": "user",
                        "email": "user@example.com",
                    }))
                    .into_response()
                }
            }),
        );

    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind mock IdP");
    let addr = listener.local_addr().expect("local addr");
    let base_url = format!("http://127.0.0.1:{}", addr.port());

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                rx.await.ok();
            })
            .await
            .ok();
    });

    (base_url, tx)
}

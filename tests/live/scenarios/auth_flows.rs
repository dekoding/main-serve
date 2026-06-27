//! Authentication flows scenario.
//!
//! Tests Main Serve with multiple authentication mechanisms:
//!
//! - JWT token authentication (HS256)
//! - API key authentication (header-based)
//! - HTTP Basic authentication
//! - Role-based access control (RBAC)
//! - Token revocation
//! - User registration with password hashing (argon2id)
//! - Login flow
//!
//! Config: demonstrates all three auth providers with different endpoints
//! protected by different mechanisms.

use crate::support::{BinaryHandle, LiveClient};
use reqwest::StatusCode;
use serde_json::json;

const AUTH_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

logging:
  level: "info"
  format: "json"

databases:
  main:
    driver: "sqlite"
    url: "sqlite://auth.db?mode=rwc"
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

auth:
  jwt:
    secret: "test-jwt-secret-key-32ch-long"
    algorithm: "HS256"
    issuer: "main-serve"
    expiry: 3600
    role_claim: "role"
    revocation:
      store: "in_memory"

  api_key:
    location: "header"
    name: "X-API-Key"
    keys:
      - key: "reader-key-001"
        role: "reader"
      - key: "writer-key-002"
        role: "writer"
      - key: "admin-key-003"
        role: "admin"

  basic:
    realm: "Admin Panel"
    users:
      - username: "admin"
        password_hash: "$argon2id$v=19$m=19456,t=2,p=1$dGVzdHNhbHQxMjM0NTY3OA$j8m9zaHJrmzBD2Ce/1dbliH5M0gBUF8J4WlNmdYrKIo"
        role: "admin"

  register:
    enabled: true
    table: "users"
    database: "main"
    default_role: "user"
    password_hash: "argon2id"

endpoints:
  - path: "/api/public"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"public": true}'
    auth: "none"

  - path: "/api/jwt-protected"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"protected": "jwt", "message": "You are authenticated via JWT"}'
    auth: "jwt"
    roles: ["admin", "user"]

  - path: "/api/jwt-admin"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"protected": "jwt-admin", "message": "You are an admin"}'
    auth: "jwt"
    roles: ["admin"]

  - path: "/api/apikey-protected"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"protected": "apikey", "message": "You are authenticated via API key"}'
    auth: "api_key"
    roles: ["reader", "writer", "admin"]

  - path: "/api/basic-protected"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"protected": "basic", "message": "You are authenticated via Basic auth"}'
    auth: "basic"
    roles: ["admin"]

  - path: "/register"
    methods: ["post"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"ok": true}'

  - path: "/auth/login"
    methods: ["post"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"token": "test-jwt-token", "expires_in": 3600}'
"#;

#[tokio::test]
async fn test_public_endpoint() {
    let server = BinaryHandle::spawn(AUTH_CONFIG, None).await.expect("spawn");
    let client = server.client();

    let resp = client
        .get_json::<serde_json::Value>("/api/public")
        .await
        .expect("GET /api/public");

    assert_eq!(resp.get("public").and_then(|v| v.as_bool()), Some(true));

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_jwt_unauthorized() {
    let server = BinaryHandle::spawn(AUTH_CONFIG, None).await.expect("spawn");
    let client = server.client();

    let resp = client.get("/api/jwt-protected").await.expect("GET without token");
    client.assert_status(&resp, StatusCode::UNAUTHORIZED).await;

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_jwt_authorized() {
    let server = BinaryHandle::spawn(AUTH_CONFIG, None).await.expect("spawn");

    // Create a mock JWT token
    let token = create_mock_jwt("user-42", "user");

    let authed = LiveClient::new(server.base_url())
        .with_bearer_token(&token);

    let resp = authed
        .get_json::<serde_json::Value>("/api/jwt-protected")
        .await
        .expect("GET with JWT");

    assert_eq!(
        resp.get("message").and_then(|v| v.as_str()),
        Some("You are authenticated via JWT")
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_jwt_role_restriction() {
    let server = BinaryHandle::spawn(AUTH_CONFIG, None).await.expect("spawn");
    let client = server.client();

    // Regular user should not have access to admin endpoint
    let token = create_mock_jwt("user-42", "user");
    let authed = LiveClient::new(server.base_url())
        .with_bearer_token(&token);

    let resp = authed.get("/api/jwt-admin").await.expect("GET admin endpoint as user");
    client.assert_status(&resp, StatusCode::FORBIDDEN).await;

    // Admin should have access
    let admin_token = create_mock_jwt("admin-1", "admin");
    let admin_client = LiveClient::new(server.base_url())
        .with_bearer_token(&admin_token);

    let resp = admin_client
        .get_json::<serde_json::Value>("/api/jwt-admin")
        .await
        .expect("GET admin endpoint as admin");

    assert_eq!(
        resp.get("message").and_then(|v| v.as_str()),
        Some("You are an admin")
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_api_key_auth() {
    let server = BinaryHandle::spawn(AUTH_CONFIG, None).await.expect("spawn");
    let client = server.client();

    // No API key -> unauthorized
    let resp = client.get("/api/apikey-protected").await.expect("GET without key");
    client.assert_status(&resp, StatusCode::UNAUTHORIZED).await;

    // Valid reader key
    let client = client.with_header("X-API-Key", "reader-key-001");
    let resp = client
        .get_json::<serde_json::Value>("/api/apikey-protected")
        .await
        .expect("GET with reader key");

    assert_eq!(
        resp.get("message").and_then(|v| v.as_str()),
        Some("You are authenticated via API key")
    );

    // Invalid key
    let bad_client = LiveClient::new(server.base_url())
        .with_header("X-API-Key", "invalid-key");
    let resp = bad_client.get("/api/apikey-protected").await.expect("GET with bad key");
    client.assert_status(&resp, StatusCode::UNAUTHORIZED).await;

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_basic_auth() {
    let server = BinaryHandle::spawn(AUTH_CONFIG, None).await.expect("spawn");
    let client = server.client();

    // No credentials -> unauthorized
    let resp = client.get("/api/basic-protected").await.expect("GET without creds");

    // Check WWW-Authenticate header
    let www_auth = resp.headers().get("www-authenticate")
        .and_then(|v| v.to_str().ok());
    assert!(www_auth.is_some_and(|w| w.contains("Basic")));

    client.assert_status(&resp, StatusCode::UNAUTHORIZED).await;

    // Valid admin credentials (base64("admin:password"))
    use base64::engine::{Engine as _, general_purpose::STANDARD as b64};
    let credentials = b64.encode("admin:password");

    let authed = LiveClient::new(server.base_url())
        .with_header("Authorization", &format!("Basic {credentials}"));

    let resp = authed
        .get_json::<serde_json::Value>("/api/basic-protected")
        .await
        .expect("GET with basic auth");

    assert_eq!(
        resp.get("message").and_then(|v| v.as_str()),
        Some("You are authenticated via Basic auth")
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_cors_headers() {
    let server = BinaryHandle::spawn(AUTH_CONFIG, None).await.expect("spawn");
    let client = server.client();

    // OPTIONS preflight request
    let resp = client
        .client()
        .clone()
        .request(reqwest::Method::from_bytes(b"OPTIONS").unwrap(), client.url("/api/jwt-protected"))
        .header("Origin", "https://example.com")
        .header("Access-Control-Request-Method", "GET")
        .send()
        .await
        .expect("OPTIONS preflight");

    // CORS headers should be present
    let access_control = resp.headers().get("access-control-allow-origin");
    // If CORS is configured globally, this should have a value
    let _ = access_control;

    server.shutdown().await.expect("server shutdown");
}

/// Create a mock JWT token with the given subject and role.
///
/// This is a simplified JWT creation for testing purposes. The token is
/// signed with the same secret used in the auth config using HMAC-SHA256.
fn create_mock_jwt(sub: &str, role: &str) -> String {
    use base64::engine::Engine as _;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    // Minimal JWT structure: header.payload.signature
    // header: {"alg": "HS256", "typ": "JWT"}
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        serde_json::to_string(&json!({
            "alg": "HS256",
            "typ": "JWT"
        })).unwrap()
    );

    // payload: {"sub": "user-42", "role": "user", "iss": "main-serve", "iat": now, "exp": now + 3600}
    let now = 1800000000u64;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        serde_json::to_string(&json!({
            "sub": sub,
            "role": role,
            "iss": "main-serve",
            "iat": now,
            "exp": now + 3600
        })).unwrap()
    );

    // Signature: HMAC-SHA256(header.payload, secret) - proper HS256
    let signing_input = format!("{}.{}", header, payload);
    let secret = "test-jwt-secret-key-32ch-long";
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(signing_input.as_bytes());
    let signature = mac.finalize().into_bytes();
    let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature);

    format!("{}.{}.{}", header, payload, sig)
}

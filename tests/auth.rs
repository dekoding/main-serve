/// Authentication mechanism integration tests.
///
/// Tests JWT, API key, and Basic auth enforcement on endpoints,
/// including role-based authorization.
mod support;

use std::io::Write;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use main_serve::config::load_config;
use main_serve::config::types::JwtConfig;
use main_serve::middleware::auth::validators::jwt::create_token;

use support::db::{TestDatabase, enabled_backends};
use support::{json_body, setup_server, start_mock_idp};

use crate::support::db::TestBackend;

fn jwt_config() -> JwtConfig {
    JwtConfig {
        secret: "test-jwt-secret-key-long-enough".to_string(),
        algorithm: main_serve::config::types::JwtAlgorithm::HS256,
        issuer: "test-issuer".to_string(),
        audience: "test-audience".to_string(),
        expiry: 3600,
        role_claim: "role".to_string(),
    }
}

// =============================================================================
// JWT Auth
// =============================================================================

const JWT_CONFIG: &str = r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-jwt-secret-key-long-enough"
    algorithm: "HS256"
    issuer: "test-issuer"
    audience: "test-audience"
    expiry: 3600
    role_claim: "role"

endpoints:
  - path: "/api/public"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"msg": "public"}'
    auth: "none"

  - path: "/api/private"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"msg": "private"}'
    auth: "jwt"

  - path: "/api/admin"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"msg": "admin only"}'
    auth: "jwt"
    roles: ["admin"]
"#;

#[tokio::test]
async fn test_public_endpoint_needs_no_auth() {
    let (app, _f) = setup_server(JWT_CONFIG).await;

    let req = Request::builder()
        .uri("/api/public")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_jwt_required_no_token() {
    let (app, _f) = setup_server(JWT_CONFIG).await;

    let req = Request::builder()
        .uri("/api/private")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_jwt_required_invalid_token() {
    let (app, _f) = setup_server(JWT_CONFIG).await;

    let req = Request::builder()
        .uri("/api/private")
        .header("authorization", "Bearer invalid.token.here")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_jwt_valid_token() {
    let (app, _f) = setup_server(JWT_CONFIG).await;

    let token = create_token("user1", Some("viewer"), &jwt_config()).unwrap();

    let req = Request::builder()
        .uri("/api/private")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_jwt_role_required_correct_role() {
    let (app, _f) = setup_server(JWT_CONFIG).await;

    let token = create_token("admin1", Some("admin"), &jwt_config()).unwrap();

    let req = Request::builder()
        .uri("/api/admin")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_jwt_role_required_wrong_role() {
    let (app, _f) = setup_server(JWT_CONFIG).await;

    let token = create_token("user1", Some("viewer"), &jwt_config()).unwrap();

    let req = Request::builder()
        .uri("/api/admin")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let json = json_body(response).await;
    assert_eq!(json["error"]["code"], "forbidden");
}

#[tokio::test]
async fn test_jwt_role_required_no_role() {
    let (app, _f) = setup_server(JWT_CONFIG).await;

    let token = create_token("user1", None, &jwt_config()).unwrap();

    let req = Request::builder()
        .uri("/api/admin")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// =============================================================================
// API Key Auth
// =============================================================================

const API_KEY_CONFIG: &str = r#"
server:
  port: 0

auth:
  api_key:
    location: "header"
    name: "X-API-Key"
    keys:
      - key: "secret-api-key-1"
        role: "admin"
      - key: "secret-api-key-2"

endpoints:
  - path: "/api/data"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"data": "secured"}'
    auth: "api_key"

  - path: "/api/admin-data"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"data": "admin"}'
    auth: "api_key"
    roles: ["admin"]
"#;

#[tokio::test]
async fn test_api_key_missing() {
    let (app, _f) = setup_server(API_KEY_CONFIG).await;

    let req = Request::builder()
        .uri("/api/data")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_api_key_invalid() {
    let (app, _f) = setup_server(API_KEY_CONFIG).await;

    let req = Request::builder()
        .uri("/api/data")
        .header("X-API-Key", "wrong-key")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_api_key_valid() {
    let (app, _f) = setup_server(API_KEY_CONFIG).await;

    let req = Request::builder()
        .uri("/api/data")
        .header("X-API-Key", "secret-api-key-1")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_api_key_valid_no_role_key() {
    let (app, _f) = setup_server(API_KEY_CONFIG).await;

    let req = Request::builder()
        .uri("/api/data")
        .header("X-API-Key", "secret-api-key-2")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_api_key_role_check_pass() {
    let (app, _f) = setup_server(API_KEY_CONFIG).await;

    let req = Request::builder()
        .uri("/api/admin-data")
        .header("X-API-Key", "secret-api-key-1")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_api_key_role_check_fail() {
    let (app, _f) = setup_server(API_KEY_CONFIG).await;

    // Key 2 has no role, should be forbidden on admin endpoint.
    let req = Request::builder()
        .uri("/api/admin-data")
        .header("X-API-Key", "secret-api-key-2")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// =============================================================================
// HTTP Basic Auth
// =============================================================================

#[tokio::test]
async fn test_basic_auth_missing() {
    // Need to generate a password hash dynamically for the config.
    let yaml = basic_auth_yaml();
    let (app, _f) = setup_server(&yaml).await;

    let req = Request::builder()
        .uri("/api/secure")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_basic_auth_valid() {
    let yaml = basic_auth_yaml();
    let (app, _f) = setup_server(&yaml).await;

    // "testuser:testpass" in Base64 = "dGVzdHVzZXI6dGVzdHBhc3M="
    let req = Request::builder()
        .uri("/api/secure")
        .header("authorization", "Basic dGVzdHVzZXI6dGVzdHBhc3M=")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_basic_auth_wrong_password() {
    let yaml = basic_auth_yaml();
    let (app, _f) = setup_server(&yaml).await;

    // "testuser:wrongpass" in Base64 = "dGVzdHVzZXI6d3JvbmdwYXNz"
    let req = Request::builder()
        .uri("/api/secure")
        .header("authorization", "Basic dGVzdHVzZXI6d3JvbmdwYXNz")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_basic_auth_realm_header() {
    let yaml = basic_auth_yaml();
    let (app, _f) = setup_server(&yaml).await;

    // Missing credentials should return 401 with WWW-Authenticate: Basic realm="test"
    let req = Request::builder()
        .uri("/api/secure")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let www_auth = response
        .headers()
        .get("www-authenticate")
        .expect("WWW-Authenticate header should be present")
        .to_str()
        .unwrap();
    assert_eq!(www_auth, "Basic realm=\"test\"");
}

/// Generate Basic auth YAML config with a properly hashed password.
fn basic_auth_yaml() -> String {
    use argon2::{Argon2, PasswordHasher, password_hash::SaltString};

    let salt = SaltString::from_b64("dGVzdHNhbHR2YWx1ZQ").unwrap();
    let hash = Argon2::default()
        .hash_password(b"testpass", &salt)
        .unwrap()
        .to_string();

    format!(
        r#"
server:
  port: 0

auth:
  basic:
    realm: "test"
    users:
      - username: "testuser"
        password_hash: "{hash}"
        role: "user"

endpoints:
  - path: "/api/secure"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{{"msg": "secure"}}'
    auth: "basic"
"#
    )
}

const JWT_CRUD_CONFIG: &str = r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-jwt-secret-key-long-enough"
    algorithm: "HS256"
    issuer: "test-issuer"
    audience: "test-audience"

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
      - name: "name"
        type: "text"

endpoints:
  - path: "/api/items"
    methods: ["get"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
    auth: "jwt"
"#;

const API_KEY_CRUD_CONFIG: &str = r#"
server:
  port: 0

auth:
  api_key:
    location: "header"
    name: "X-API-Key"
    keys:
      - key: "secret-api-key-1"
        role: "admin"

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
      - name: "name"
        type: "text"

endpoints:
  - path: "/api/items"
    methods: ["get"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
    auth: "api_key"
"#;

// =============================================================================
// Mixed: auth on CRUD endpoints
// =============================================================================

#[tokio::test]
async fn test_jwt_auth_on_crud_endpoint() {
    let test_db = TestDatabase::new(TestBackend::Sqlite, "auth_jwt_crud");
    let (app, _state, _pools) = test_db.setup_app(JWT_CRUD_CONFIG, "jwt_crud.yaml").await;

    // Without token -> 401.
    let req = Request::builder()
        .uri("/api/items")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // With valid token -> 200.
    let token = create_token("user1", None, &jwt_config()).unwrap();
    let req = Request::builder()
        .uri("/api/items")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_jwt_auth_on_crud_endpoint_across_backends() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "auth_jwt_crud");
        let (app, _state, pools) = test_db.setup_app(JWT_CRUD_CONFIG, "jwt_crud.yaml").await;

        let req = Request::builder()
            .uri("/api/items")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "backend: {backend}"
        );

        let token = create_token("user1", None, &jwt_config()).unwrap();
        let req = Request::builder()
            .uri("/api/items")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");

        // Clean up tables for non-SQLite backends
        test_db.cleanup(&pools).await;
    }
}

#[tokio::test]
async fn test_api_key_auth_on_crud_endpoint_across_backends() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "auth_api_key_crud");
        let (app, _state, pools) = test_db
            .setup_app(API_KEY_CRUD_CONFIG, "api_key_crud.yaml")
            .await;

        let req = Request::builder()
            .uri("/api/items")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "backend: {backend}"
        );

        let req = Request::builder()
            .uri("/api/items")
            .header("X-API-Key", "secret-api-key-1")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");

        // Clean up tables for non-SQLite backends
        test_db.cleanup(&pools).await;
    }
}

// =============================================================================
// OAuth2 Code Flow
// =============================================================================

fn oauth2_config_yaml(idp_url: &str) -> String {
    format!(
        r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-jwt-secret-key-long-enough"
    algorithm: "HS256"
    issuer: "test-issuer"
    audience: "test-audience"
    expiry: 3600
  oauth2:
    provider: "test-idp"
    authorization_url: "{idp_url}/authorize"
    token_url: "{idp_url}/token"
    userinfo_url: "{idp_url}/userinfo"
    client_id: "test-client-id"
    client_secret: "test-client-secret"
    scopes:
      - "openid"
      - "profile"
    redirect_url: "http://localhost:8080/_main-serve/oauth2/callback"
    success_url: "/dashboard"
    cookie_name: "test_token"

endpoints:
  - path: "/api/protected"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{{"msg": "protected"}}'
    auth: "jwt"
"#
    )
}

#[tokio::test]
async fn test_oauth2_authorize_redirect() {
    let (idp_url, _shutdown) = start_mock_idp().await;
    let yaml = oauth2_config_yaml(&idp_url);
    let (app, _f) = setup_server(&yaml).await;

    let req = Request::builder()
        .uri("/_main-serve/oauth2/authorize")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();

    assert_eq!(response.status(), StatusCode::FOUND);

    let location = response
        .headers()
        .get("location")
        .expect("location header")
        .to_str()
        .unwrap();

    // Should redirect to the IdP's authorization URL with correct params.
    assert!(location.starts_with(&format!("{idp_url}/authorize?")));
    assert!(location.contains("response_type=code"));
    assert!(location.contains("client_id=test-client-id"));
    assert!(location.contains("redirect_uri="));
    assert!(location.contains("code_challenge="));
    assert!(location.contains("code_challenge_method=S256"));
    assert!(location.contains("scope=openid+profile"));
    assert!(location.contains("state="));
}

#[tokio::test]
async fn test_oauth2_callback_missing_code() {
    let (idp_url, _shutdown) = start_mock_idp().await;
    let yaml = oauth2_config_yaml(&idp_url);
    let (app, _f) = setup_server(&yaml).await;

    let req = Request::builder()
        .uri("/_main-serve/oauth2/callback?state=some-state")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_oauth2_callback_invalid_state() {
    let (idp_url, _shutdown) = start_mock_idp().await;
    let yaml = oauth2_config_yaml(&idp_url);
    let (app, _f) = setup_server(&yaml).await;

    let req = Request::builder()
        .uri("/_main-serve/oauth2/callback?code=test-code&state=bogus-state")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_oauth2_callback_idp_error_response() {
    let (idp_url, _shutdown) = start_mock_idp().await;
    let yaml = oauth2_config_yaml(&idp_url);
    let (app, _f) = setup_server(&yaml).await;

    let req = Request::builder()
        .uri(
            "/_main-serve/oauth2/callback?error=access_denied&error_description=User+denied+access",
        )
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = json_body(response).await;
    let msg = body["error"]["message"].as_str().unwrap();
    assert!(msg.contains("OAuth2 authorization failed"));
}

#[tokio::test]
async fn test_oauth2_full_code_flow() {
    let (idp_url, shutdown) = start_mock_idp().await;
    let yaml = oauth2_config_yaml(&idp_url);

    let (app, _f) = setup_server(&yaml).await;

    // Step 1: Hit /authorize to get the redirect URL.
    let req = Request::builder()
        .uri("/_main-serve/oauth2/authorize")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);

    let location = response
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap();

    // Extract the state parameter from the redirect URL.
    let url = url::Url::parse(location).unwrap();
    let state_param = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.to_string())
        .expect("state param in redirect URL");

    // Step 2: Simulate the IdP redirecting back with a code + state.
    let callback_uri =
        format!("/_main-serve/oauth2/callback?code=test-auth-code&state={state_param}");
    let req = Request::builder()
        .uri(&callback_uri)
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);

    // Should redirect to success_url.
    let redirect = response
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(redirect, "/dashboard");

    // Should set the JWT cookie.
    let set_cookie = response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(set_cookie.starts_with("test_token="));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Lax"));
    assert!(set_cookie.contains("Path=/"));
    assert!(set_cookie.contains("Max-Age=3600"));
    // No TLS configured, so no Secure flag.
    assert!(!set_cookie.contains("Secure"));

    // Extract the JWT from the cookie.
    let jwt = set_cookie
        .split('=')
        .nth(1)
        .unwrap()
        .split(';')
        .next()
        .unwrap();

    // Step 3: Use the JWT cookie to access a protected endpoint.
    let req = Request::builder()
        .uri("/api/protected")
        .header("cookie", format!("test_token={jwt}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Step 4: Verify the JWT also works via Authorization header.
    let req = Request::builder()
        .uri("/api/protected")
        .header("authorization", format!("Bearer {jwt}"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Cleanup mock IdP.
    shutdown.send(()).ok();
}

#[tokio::test]
async fn test_oauth2_state_is_one_time_use() {
    let (idp_url, shutdown) = start_mock_idp().await;
    let yaml = oauth2_config_yaml(&idp_url);

    let (app, _f) = setup_server(&yaml).await;

    // Get a valid state from /authorize.
    let req = Request::builder()
        .uri("/_main-serve/oauth2/authorize")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let location = response
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap();
    let url = url::Url::parse(location).unwrap();
    let state_param = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.to_string())
        .unwrap();

    // First callback should succeed.
    let uri = format!("/_main-serve/oauth2/callback?code=test-code&state={state_param}");
    let req = Request::builder().uri(&uri).body(Body::empty()).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);

    // Second callback with same state should fail (one-time use).
    let req = Request::builder().uri(&uri).body(Body::empty()).unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    shutdown.send(()).ok();
}

#[tokio::test]
async fn test_jwt_auth_via_cookie_fallback() {
    let yaml = r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-jwt-secret-key-long-enough"
    algorithm: "HS256"
    issuer: "test-issuer"
    audience: "test-audience"
    expiry: 3600
  oauth2:
    cookie_name: "my_auth_cookie"

endpoints:
  - path: "/api/secure"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"msg": "secure"}'
    auth: "jwt"
"#;

    let (app, _f) = setup_server(yaml).await;

    let token = create_token("cookie-user", Some("admin"), &jwt_config()).unwrap();

    // Without any auth -> 401.
    let req = Request::builder()
        .uri("/api/secure")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // With JWT in cookie -> 200.
    let req = Request::builder()
        .uri("/api/secure")
        .header("cookie", format!("my_auth_cookie={token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // With JWT in Authorization header -> 200 (still works).
    let req = Request::builder()
        .uri("/api/secure")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_oauth2_code_flow_requires_jwt_config() {
    let yaml = r#"
server:
  port: 0

auth:
  oauth2:
    authorization_url: "https://idp.example.com/authorize"
    token_url: "https://idp.example.com/token"
    client_id: "my-client"
    redirect_url: "http://localhost:8080/_main-serve/oauth2/callback"

endpoints: []
"#;

    let mut f = tempfile::NamedTempFile::new().expect("tempfile");
    f.write_all(yaml.as_bytes()).expect("write");
    let result = load_config(f.path());

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("auth.jwt must be configured"),
        "Expected JWT requirement error, got: {err}"
    );
}

#[tokio::test]
async fn test_oauth2_endpoints_not_registered_without_config() {
    // No OAuth2 configured -> OAuth2 endpoints should not exist.
    let yaml = r#"
server:
  port: 0

endpoints:
  - path: "/api/public"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"msg": "public"}'
    auth: "none"
"#;

    let (app, _f) = setup_server(yaml).await;

    let req = Request::builder()
        .uri("/_main-serve/oauth2/authorize")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    // Should 404 since OAuth2 is not configured.
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let req = Request::builder()
        .uri("/_main-serve/oauth2/callback?code=x&state=y")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// =============================================================================
// API Key Query Parameter Location
// =============================================================================

#[tokio::test]
async fn test_api_key_query_param_on_custom_response() {
    let yaml = r#"
server:
  port: 0

auth:
  api_key:
    location: "query"
    name: "api_key"
    keys:
      - key: "secret-query-key"
        role: "admin"

endpoints:
  - path: "/api/data"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"data": "secured"}'
    auth: "api_key"
"#;

    let (app, _f) = setup_server(yaml).await;

    // Without key -> 401.
    let req = Request::builder()
        .uri("/api/data")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // With key in query string -> should be 200.
    let req = Request::builder()
        .uri("/api/data?api_key=secret-query-key")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "API key in query param should authenticate on custom_response endpoints"
    );
}

// =============================================================================
// OAuth2 Token Introspection (auth: "oauth2")
// =============================================================================

#[tokio::test]
async fn test_oauth2_token_introspection_on_endpoint() {
    let (idp_url, shutdown) = start_mock_idp().await;

    let yaml = format!(
        r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-jwt-secret-key-long-enough"
    algorithm: "HS256"
    issuer: "test-issuer"
    audience: "test-audience"
  oauth2:
    userinfo_url: "{idp_url}/userinfo"
    client_id: "test-client"
    client_secret: "test-secret"
    redirect_url: "http://localhost:8080/_main-serve/oauth2/callback"

endpoints:
  - path: "/api/userdata"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{{"data": "user-specific"}}'
    auth: "oauth2"
"#
    );

    let (app, _f) = setup_server(&yaml).await;

    // Without token -> 401.
    let req = Request::builder()
        .uri("/api/userdata")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // With a valid Bearer token that the mock IdP's /userinfo accepts -> 200.
    let req = Request::builder()
        .uri("/api/userdata")
        .header("authorization", "Bearer mock-access-token-123")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "OAuth2 token introspection via userinfo should authenticate the request"
    );

    shutdown.send(()).ok();
}

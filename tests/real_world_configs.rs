/// Integration tests with realistic, comprehensive YAML configurations.
///
/// Each test scenario validates that a full real-world config loads,
/// validates, and serves requests correctly. Configs live in
/// `tests/configs/` as standalone YAML files with JSON schema references.
///
/// Scenarios:
///   1. Static file serving (with and without SPA fallback)
///   2. Custom responses of various types
///   3. API endpoints protected by each auth variety
///   4. Combined config: static files + protected CRUD endpoints
///   5. OAuth2 / OIDC: token introspection with role enforcement
mod support;

use std::sync::LazyLock;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use main_serve::config::load_config;
use main_serve::config::types::JwtConfig;
use main_serve::middleware::auth::validators::jwt::create_token;
use main_serve::server::{AppState, build_router};

use support::db::{TestDatabase, create_pools_and_migrate, enabled_backends};
use support::helpers::jwt_config;
use support::{json_body, setup_server, start_mock_idp};

// =============================================================================
// Helpers
// =============================================================================

/// Read a config template file (relative to the project root's config/templates/).
fn read_config(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("config/templates")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()))
}

// Each config file is read from disk once and cached for the entire test binary.
static STATIC_FILES_YAML: LazyLock<String> = LazyLock::new(|| read_config("static_files.yaml"));
static CUSTOM_RESPONSES_YAML: LazyLock<String> =
    LazyLock::new(|| read_config("custom_responses.yaml"));
static AUTH_PROTECTED_YAML: LazyLock<String> = LazyLock::new(|| read_config("auth_protected.yaml"));
static COMBINED_YAML: LazyLock<String> = LazyLock::new(|| read_config("combined.yaml"));
static OAUTH2_YAML: LazyLock<String> = LazyLock::new(|| read_config("oauth2.yaml"));
static PROXY_YAML: LazyLock<String> = LazyLock::new(|| read_config("proxy.yaml"));
static CRUD_API_YAML: LazyLock<String> = LazyLock::new(|| read_config("crud_api.yaml"));

fn basic_auth_hash() -> String {
    use argon2::{Argon2, PasswordHasher, password_hash::SaltString};

    let salt = SaltString::from_b64("dGVzdHNhbHR2YWx1ZQ").unwrap();
    Argon2::default()
        .hash_password(b"s3cureP@ss", &salt)
        .unwrap()
        .to_string()
}

/// Populate a directory with test site files. Returns the root path.
fn write_site_files(dir: &std::path::Path, files: &[(&str, &str)]) -> std::path::PathBuf {
    let root = dir.join("public");
    std::fs::create_dir_all(&root).unwrap();
    for (path, content) in files {
        let file_path = root.join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&file_path, content).unwrap();
    }
    root
}

/// Set up a server from a config template with static files.
/// Replaces `{ROOT}` with the temp directory path.
/// Returns (router, TempDir) - caller must keep TempDir alive.
async fn setup_static_server(
    yaml: &str,
    files: &[(&str, &str)],
) -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = write_site_files(dir.path(), files);

    let yaml = yaml.replace("{ROOT}", &root.display().to_string());
    let config_file = dir.path().join("config.yaml");
    std::fs::write(&config_file, &yaml).unwrap();

    let config = load_config(&config_file).expect("load config");
    let state = AppState::new(config, config_file, "test-token".to_string());
    let config_guard = state.config.read().await;
    let app = build_router(&config_guard, state.clone()).await;
    drop(config_guard);

    (app, dir)
}

/// Set up a combined scenario with both static files and a DB-backed config.
async fn setup_combined_app(
    test_db: &TestDatabase,
    yaml_template: &str,
    site_files: &[(&str, &str)],
) -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = write_site_files(dir.path(), site_files);

    let yaml = test_db
        .render_yaml(yaml_template)
        .replace("{ROOT}", &root.display().to_string())
        .replace("__BASIC_AUTH_HASH__", &basic_auth_hash());
    let config_path = dir.path().join("config.yaml");
    std::fs::write(&config_path, &yaml).unwrap();

    let config = load_config(&config_path).expect("load config");
    let pools = create_pools_and_migrate(&config).await;

    let state = AppState::new(config, config_path, "test-token".to_string());
    {
        let mut pool_lock = state.db_pools.write().await;
        *pool_lock = pools;
    }
    let config_guard = state.config.read().await;
    let app = build_router(&config_guard, state.clone()).await;
    drop(config_guard);

    (app, dir)
}

const SITE_FILES: &[(&str, &str)] = &[
    (
        "index.html",
        "<!DOCTYPE html><html><head><title>My App</title></head><body><div id='app'></div></body></html>",
    ),
    (
        "style.css",
        "body { margin: 0; font-family: sans-serif; } .container { max-width: 1200px; }",
    ),
    (
        "app.js",
        "document.addEventListener('DOMContentLoaded', () => { console.log('loaded'); });",
    ),
    ("images/logo.png", "FAKE_PNG_CONTENT"),
    ("data/report.json", r#"{"title":"Q4 Report","entries":42}"#),
    (
        "sub/index.html",
        "<html><body>Subdirectory Index</body></html>",
    ),
];

const COMBINED_SITE_FILES: &[(&str, &str)] = &[
    (
        "index.html",
        "<!DOCTYPE html><html><head><title>MyApp</title><link rel='stylesheet' href='/assets/style.css'></head><body><div id='root'></div><script src='/assets/bundle.js'></script></body></html>",
    ),
    (
        "style.css",
        ".root { display: flex; } .sidebar { width: 250px; }",
    ),
    (
        "bundle.js",
        "const app = { init() { console.log('MyApp v1.0'); } }; app.init();",
    ),
    ("favicon.ico", "FAKE_ICO_DATA"),
];

// =============================================================================
// Scenario 1: Static File Serving (with and without SPA fallback)
// =============================================================================

#[tokio::test]
async fn test_static_standard_serves_existing_files() {
    let yaml = &*STATIC_FILES_YAML;
    let (app, _dir) = setup_static_server(yaml, SITE_FILES).await;

    // Serve index.html at root
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/assets/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("My App"));

    // Serve CSS with correct content-type
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/assets/style.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("text/css"), "Expected text/css, got: {ct}");

    // Serve JS file
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/assets/app.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("DOMContentLoaded"));

    // Serve nested file
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/assets/data/report.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("Q4 Report"));

    // Serve subdirectory index
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/assets/sub/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("Subdirectory Index"));

    // Cache header present
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/assets/style.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let cc = resp
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        cc.contains("max-age=86400"),
        "Expected max-age=86400, got: {cc}"
    );
}

#[tokio::test]
async fn test_static_standard_404_for_missing() {
    let yaml = &*STATIC_FILES_YAML;
    let (app, _dir) = setup_static_server(yaml, SITE_FILES).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/assets/nonexistent.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/assets/deep/nested/path")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_static_spa_returns_index_for_unknown_routes() {
    let yaml = &*STATIC_FILES_YAML;
    let (app, _dir) = setup_static_server(yaml, SITE_FILES).await;

    // Known file still works
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/app/style.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("text/css"));

    // Unknown deep route returns index.html (SPA client-side routing)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/app/dashboard/settings/profile")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(
        body.contains("My App"),
        "SPA fallback should return index.html"
    );

    // Root also works
    let resp = app
        .clone()
        .oneshot(Request::builder().uri("/app/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("My App"));

    // SPA mode should have cache_max_age: 0 (no caching for SPA routes)
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/app/some/route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    if let Some(cc) = resp.headers().get("cache-control") {
        let cc = cc.to_str().unwrap();
        assert!(
            cc.contains("max-age=0") || cc.contains("no-cache"),
            "Unexpected cache-control: {cc}"
        );
    }
}

#[tokio::test]
async fn test_static_directory_listing() {
    let yaml = &*STATIC_FILES_YAML;
    let (app, _dir) = setup_static_server(yaml, SITE_FILES).await;

    // /files/images/ has no index.html - should show listing
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/files/images/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(
        body.contains("logo.png"),
        "Directory listing should contain logo.png"
    );

    // /files/data/ has no index.html - should list report.json
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/files/data/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(
        body.contains("report.json"),
        "Directory listing should contain report.json"
    );
}

// =============================================================================
// Scenario 2: Custom Responses of Various Types
// =============================================================================

#[tokio::test]
async fn test_custom_response_json_api_info() {
    let yaml = &*CUSTOM_RESPONSES_YAML;
    let (app, _f) = setup_server(yaml).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/info")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        ct.contains("application/json"),
        "Expected JSON content-type, got: {ct}"
    );

    let svc = resp
        .headers()
        .get("x-service-name")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(svc, "my-service");

    let ver = resp
        .headers()
        .get("x-api-version")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ver, "2.1");

    let json = json_body(resp).await;
    assert_eq!(json["name"], "My Service");
    assert_eq!(json["version"], "2.1.0");
    assert_eq!(json["environment"], "production");
}

#[tokio::test]
async fn test_custom_response_health_check() {
    let yaml = &*CUSTOM_RESPONSES_YAML;
    let (app, _f) = setup_server(yaml).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = json_body(resp).await;
    assert_eq!(json["status"], "healthy");
}

#[tokio::test]
async fn test_custom_response_html_page() {
    let yaml = &*CUSTOM_RESPONSES_YAML;
    let (app, _f) = setup_server(yaml).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/welcome")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("text/html"), "Expected text/html, got: {ct}");

    let body = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("<h1>Welcome to My Service</h1>"));
    assert!(body.contains("API docs at /api/info"));
}

#[tokio::test]
async fn test_custom_response_plain_text() {
    let yaml = &*CUSTOM_RESPONSES_YAML;
    let (app, _f) = setup_server(yaml).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/robots.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("text/plain"), "Expected text/plain, got: {ct}");

    let body = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("User-agent: *"));
    assert!(body.contains("Disallow: /api/"));
}

#[tokio::test]
async fn test_custom_response_redirect() {
    let yaml = &*CUSTOM_RESPONSES_YAML;
    let (app, _f) = setup_server(yaml).await;

    let resp = app
        .oneshot(Request::builder().uri("/docs").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::MOVED_PERMANENTLY);

    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert_eq!(location, "https://docs.example.com/v2");

    let cc = resp
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cc.contains("public"));
}

#[tokio::test]
async fn test_custom_response_service_unavailable() {
    let yaml = &*CUSTOM_RESPONSES_YAML;
    let (app, _f) = setup_server(yaml).await;

    for method in &["GET", "POST", "PUT", "DELETE"] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(*method)
                    .uri("/api/maintenance")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "method: {method}"
        );

        let retry = resp.headers().get("retry-after").unwrap().to_str().unwrap();
        assert_eq!(retry, "300");

        let json = json_body(resp).await;
        assert_eq!(json["retry_after"], 300);
    }
}

#[tokio::test]
async fn test_custom_response_xml() {
    let yaml = &*CUSTOM_RESPONSES_YAML;
    let (app, _f) = setup_server(yaml).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/sitemap.xml")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        ct.contains("application/xml"),
        "Expected application/xml, got: {ct}"
    );

    let body = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("<urlset"));
    assert!(body.contains("https://example.com/welcome"));
}

// =============================================================================
// Scenario 3: API Endpoints Protected by Each Auth Variety
// =============================================================================

/// Load the auth_protected config with the password hash placeholder filled in.
fn auth_config() -> String {
    AUTH_PROTECTED_YAML.replace("__BASIC_AUTH_HASH__", &basic_auth_hash())
}

#[tokio::test]
async fn test_auth_public_health_check_needs_no_auth() {
    let yaml = auth_config();

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_auth_health");
        let (app, _state, _pool) = test_db.setup_app(&yaml, "auth_all.yaml").await;

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");

        let json = json_body(resp).await;
        assert_eq!(json["status"], "ok");
    }
}

#[tokio::test]
async fn test_auth_jwt_crud_read_any_role() {
    let yaml_template = auth_config();

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_auth_jwt_read");
        let yaml = test_db.render_yaml(&yaml_template);
        let (app, _state, _pool) = test_db.setup_app(&yaml_template, "auth_jwt.yaml").await;

        // No token -> 401
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/articles")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "backend: {backend}"
        );

        // Valid token (any role) -> 200 for list endpoint
        let token = create_token("reader1", Some("reader"), &jwt_config(&yaml)).unwrap();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/articles")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");

        // Create an article with a non-admin token (list endpoint has no role restriction)
        let token = create_token("writer1", Some("writer"), &jwt_config(&yaml)).unwrap();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/articles")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "title": "Test Article",
                            "content": "Hello from integration test",
                            "status": "published"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "backend: {backend}");
    }
}

#[tokio::test]
async fn test_auth_jwt_crud_admin_only_endpoint() {
    let yaml_template = auth_config();

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_auth_jwt_admin");
        let yaml = test_db.render_yaml(&yaml_template);
        let (app, _state, _pool) = test_db.setup_app(&yaml_template, "auth_admin.yaml").await;

        // First create an article via the list endpoint (no role restriction)
        let admin_token = create_token("admin1", Some("admin"), &jwt_config(&yaml)).unwrap();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/articles")
                    .header("authorization", format!("Bearer {admin_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"title": "Admin Article", "content": "secret", "status": "draft"})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "backend: {backend}");

        // Fetch the created article's ID from the list endpoint.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/articles")
                    .header("authorization", format!("Bearer {admin_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list = json_body(resp).await;
        let id = list["data"][0]["id"].as_i64().unwrap();

        // Non-admin token -> 403 on single-resource endpoint
        let reader_token = create_token("reader1", Some("reader"), &jwt_config(&yaml)).unwrap();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/articles/{id}"))
                    .header("authorization", format!("Bearer {reader_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "backend: {backend}");

        // Admin token -> 200
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/articles/{id}"))
                    .header("authorization", format!("Bearer {admin_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");

        let json = json_body(resp).await;
        assert_eq!(json["title"], "Admin Article");
    }
}

#[tokio::test]
async fn test_auth_api_key_role_based_access() {
    let yaml = auth_config();

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_auth_apikey");
        let (app, _state, _pool) = test_db.setup_app(&yaml, "auth_apikey.yaml").await;

        // No key -> 401
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/feed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "backend: {backend}"
        );

        // Invalid key -> 401
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/feed")
                    .header("X-API-Key", "wrong-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "backend: {backend}"
        );

        // Reader key -> 200 on feed (reader role allowed)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/feed")
                    .header("X-API-Key", "prod-key-reader-002")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");

        // Reader key -> 403 on admin stats (admin only)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/admin/stats")
                    .header("X-API-Key", "prod-key-reader-002")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "backend: {backend}");

        // Writer key -> 403 on admin stats (admin only)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/admin/stats")
                    .header("X-API-Key", "prod-key-writer-003")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "backend: {backend}");

        // Admin key -> 200 on feed (admin also in allowed list)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/feed")
                    .header("X-API-Key", "prod-key-admin-001")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");

        // Admin key -> 200 on admin stats
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/admin/stats")
                    .header("X-API-Key", "prod-key-admin-001")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");

        let json = json_body(resp).await;
        assert_eq!(json["total_articles"], 42);
    }
}

#[tokio::test]
async fn test_auth_basic_admin_endpoint() {
    let yaml = auth_config();

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_auth_basic");
        let (app, _state, _pool) = test_db.setup_app(&yaml, "auth_basic.yaml").await;

        // No auth -> 401
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/admin/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "backend: {backend}"
        );

        // Check WWW-Authenticate header
        let www_auth = resp
            .headers()
            .get("www-authenticate")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            www_auth.contains("My Service Admin"),
            "Expected realm, got: {www_auth}"
        );

        // Wrong password -> 401
        // "admin:wrongpass" in base64 = "YWRtaW46d3JvbmdwYXNz"
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/admin/config")
                    .header("authorization", "Basic YWRtaW46d3JvbmdwYXNz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "backend: {backend}"
        );

        // Correct credentials -> 200
        // "admin:s3cureP@ss" in base64 = "YWRtaW46czNjdXJlUEBzcw=="
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/config")
                    .header("authorization", "Basic YWRtaW46czNjdXJlUEBzcw==")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");

        let json = json_body(resp).await;
        assert_eq!(json["config"], "current settings");
    }
}

// =============================================================================
// Scenario 4: Combined Config - Static Files + Protected CRUD Endpoints
// =============================================================================

#[tokio::test]
async fn test_combined_static_spa_serves_frontend() {
    let yaml = &*COMBINED_YAML;

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_combined_spa");
        let (app, _dir) = setup_combined_app(&test_db, yaml, COMBINED_SITE_FILES).await;

        // SPA index
        let resp = app
            .clone()
            .oneshot(Request::builder().uri("/app/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");
        let body = String::from_utf8(
            resp.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(body.contains("MyApp"));

        // SPA deep route falls back to index
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/app/users/123/edit")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");
        let body = String::from_utf8(
            resp.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(body.contains("MyApp"), "SPA fallback for deep route");

        // Static assets served with cache
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/assets/style.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");
        let cc = resp
            .headers()
            .get("cache-control")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cc.contains("604800"), "Expected 1 week cache, got: {cc}");
    }
}

#[tokio::test]
async fn test_combined_public_health_check() {
    let yaml = &*COMBINED_YAML;

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_combined_health");
        let (app, _dir) = setup_combined_app(&test_db, yaml, COMBINED_SITE_FILES).await;

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");

        let json = json_body(resp).await;
        assert_eq!(json["status"], "healthy");
        assert_eq!(json["version"], "1.0.0");
    }
}

#[tokio::test]
async fn test_combined_jwt_crud_full_lifecycle() {
    let yaml_template = &*COMBINED_YAML;

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_combined_crud");
        let yaml = test_db.render_yaml(yaml_template);
        let (app, _dir) = setup_combined_app(&test_db, yaml_template, COMBINED_SITE_FILES).await;

        // Unauthenticated -> 401
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/users")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "backend: {backend}"
        );

        // Create a user (JWT, any role)
        let admin_token = create_token("admin1", Some("admin"), &jwt_config(&yaml)).unwrap();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/users")
                    .header("authorization", format!("Bearer {admin_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "name": "Alice Johnson",
                            "email": "alice@example.com",
                            "role": "editor",
                            "active": true
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "backend: {backend}");

        // Fetch user ID from the list endpoint (create response format varies by backend).
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/users")
                    .header("authorization", format!("Bearer {admin_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list = json_body(resp).await;
        let user_id = list["data"][0]["id"].as_i64().unwrap();

        // List users (any JWT holder)
        let viewer_token = create_token("viewer1", Some("viewer"), &jwt_config(&yaml)).unwrap();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/users")
                    .header("authorization", format!("Bearer {viewer_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");
        let list = json_body(resp).await;
        let data = list["data"].as_array().unwrap();
        assert_eq!(data.len(), 1, "backend: {backend}");
        assert_eq!(data[0]["name"], "Alice Johnson");

        // Non-admin tries to GET single user -> 403
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/users/{user_id}"))
                    .header("authorization", format!("Bearer {viewer_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "backend: {backend}");

        // Admin updates the user
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/users/{user_id}"))
                    .header("authorization", format!("Bearer {admin_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name": "Alice J.", "email": "alice@example.com", "role": "admin", "active": true})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");

        // Admin reads updated user
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/users/{user_id}"))
                    .header("authorization", format!("Bearer {admin_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");
        let user = json_body(resp).await;
        assert_eq!(user["name"], "Alice J.");
        assert_eq!(user["role"], "admin");

        // Admin deletes the user
        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/users/{user_id}"))
                    .header("authorization", format!("Bearer {admin_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");
        let del = json_body(resp).await;
        assert_eq!(del["rows_affected"], 1);
    }
}

#[tokio::test]
async fn test_combined_api_key_service_endpoint() {
    let yaml = &*COMBINED_YAML;

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_combined_svc");
        let (app, _dir) = setup_combined_app(&test_db, yaml, COMBINED_SITE_FILES).await;

        // No key -> 401
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/service/ping")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "backend: {backend}"
        );

        // Valid service key -> 200
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/service/ping")
                    .header("X-API-Key", "service-to-service-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");

        let json = json_body(resp).await;
        assert_eq!(json["pong"], true);
    }
}

#[tokio::test]
async fn test_combined_basic_auth_admin_panel() {
    let yaml = &*COMBINED_YAML;

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_combined_admin");
        let (app, _dir) = setup_combined_app(&test_db, yaml, COMBINED_SITE_FILES).await;

        // No auth -> 401 with correct realm
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/admin/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "backend: {backend}"
        );
        let www = resp
            .headers()
            .get("www-authenticate")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            www.contains("Admin Panel"),
            "Expected 'Admin Panel' realm, got: {www}"
        );

        // Valid Basic auth -> 200
        // "admin:s3cureP@ss" -> base64 "YWRtaW46czNjdXJlUEBzcw=="
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/status")
                    .header("authorization", "Basic YWRtaW46czNjdXJlUEBzcw==")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");

        let json = json_body(resp).await;
        assert_eq!(json["admin"], true);
        assert_eq!(json["panel"], "status");
    }
}

#[tokio::test]
async fn test_combined_where_clause_filters_inactive() {
    let yaml_template = &*COMBINED_YAML;

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_combined_where");
        let yaml = test_db.render_yaml(yaml_template);
        let (app, _dir) = setup_combined_app(&test_db, yaml_template, COMBINED_SITE_FILES).await;

        let token = create_token("admin1", Some("admin"), &jwt_config(&yaml)).unwrap();

        // Create an active user
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/users")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name": "Active User", "email": "active@test.com", "active": true})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "backend: {backend}");

        // Create an inactive user
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/users")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name": "Inactive User", "email": "inactive@test.com", "active": false})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "backend: {backend}");

        // List endpoint has where_clause: "active = true" - should only return active user
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/users")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");

        let list = json_body(resp).await;
        let data = list["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            1,
            "where_clause should filter inactive users, backend: {backend}"
        );
        assert_eq!(data[0]["name"], "Active User");
    }
}

#[tokio::test]
async fn test_combined_cross_auth_isolation() {
    // Verify that auth types don't bleed across endpoints:
    // JWT token should not work on api_key endpoint, and vice versa.
    let yaml_template = &*COMBINED_YAML;

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_combined_iso");
        let yaml = test_db.render_yaml(yaml_template);
        let (app, _dir) = setup_combined_app(&test_db, yaml_template, COMBINED_SITE_FILES).await;

        // JWT token on api_key endpoint -> 401
        let jwt_token = create_token("user1", Some("service"), &jwt_config(&yaml)).unwrap();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/service/ping")
                    .header("authorization", format!("Bearer {jwt_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "JWT on api_key endpoint, backend: {backend}"
        );

        // API key on JWT endpoint -> 401
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/users")
                    .header("X-API-Key", "service-to-service-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "API key on JWT endpoint, backend: {backend}"
        );

        // Basic auth on JWT endpoint -> 401
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/users")
                    .header("authorization", "Basic YWRtaW46czNjdXJlUEBzcw==")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "Basic on JWT endpoint, backend: {backend}"
        );

        // JWT on Basic endpoint -> 401
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/status")
                    .header("authorization", format!("Bearer {jwt_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "JWT on Basic endpoint, backend: {backend}"
        );
    }
}

// =============================================================================
// Scenario 5: OAuth2 / OIDC - Token Introspection (unique to real-world configs)
//
// Core OAuth2 mechanism tests (code flow roundtrip, authorize redirect, bogus
// state, IdP error, state replay, cookie fallback) live in auth.rs.
// These tests exercise behavior specific to the external config file:
//   - Token introspection with role enforcement via a realistic IdP mock
//   - Revoked/expired token handling
//   - Auth mode independence (oauth2 vs jwt)
// =============================================================================

/// Set up a server with the OAuth2 config, replacing __IDP_URL__ with the mock.
async fn setup_oauth2_server(idp_url: &str) -> (axum::Router, tempfile::NamedTempFile) {
    let yaml = OAUTH2_YAML.replace("__IDP_URL__", idp_url);
    setup_server(&yaml).await
}

#[tokio::test]
async fn test_oauth2_public_endpoint_no_auth() {
    let (idp_url, _shutdown) = start_mock_idp().await;
    let (app, _f) = setup_oauth2_server(&idp_url).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/welcome")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("Sign in with SSO"));
    assert!(body.contains("/_main-serve/oauth2/authorize"));
}

#[tokio::test]
async fn test_oauth2_introspection_no_token_401() {
    let (idp_url, _shutdown) = start_mock_idp().await;
    let (app, _f) = setup_oauth2_server(&idp_url).await;

    // No Bearer token -> 401
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/profile")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_oauth2_introspection_valid_token() {
    let (idp_url, _shutdown) = start_mock_idp().await;
    let (app, _f) = setup_oauth2_server(&idp_url).await;

    // Valid external token -> 200 (mock always returns a valid userinfo response)
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/profile")
                .header("authorization", "Bearer some-external-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = json_body(resp).await;
    assert_eq!(json["resource"], "user profile");
}

#[tokio::test]
async fn test_oauth2_introspection_role_enforcement() {
    let (idp_url, _shutdown) = start_mock_idp().await;
    let (app, _f) = setup_oauth2_server(&idp_url).await;

    // /api/admin/settings requires role "admin".
    // Regular token -> mock returns role "user" -> 403
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/settings")
                .header("authorization", "Bearer regular-user-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Admin token -> mock returns role "admin" -> 200
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/settings")
                .header("authorization", "Bearer admin-token-xxx")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = json_body(resp).await;
    assert_eq!(json["admin"], true);
}

#[tokio::test]
async fn test_oauth2_introspection_idp_rejects_token() {
    let (idp_url, _shutdown) = start_mock_idp().await;
    let (app, _f) = setup_oauth2_server(&idp_url).await;

    // In the real world, the IdP returns 401 for expired or revoked tokens.
    // The mock rejects tokens starting with "revoked-".
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/profile")
                .header("authorization", "Bearer revoked-user-token-abc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_oauth2_and_jwt_auth_modes_are_independent() {
    let (idp_url, _shutdown) = start_mock_idp().await;
    let (app, _f) = setup_oauth2_server(&idp_url).await;

    // Demonstrates that oauth2 and jwt auth modes are independent:
    //   - oauth2 endpoints validate tokens externally (via IdP userinfo)
    //   - jwt endpoints validate tokens locally (signature + claims)
    //
    // A locally-minted JWT sent to an oauth2 endpoint gets forwarded to the
    // mock IdP's userinfo, which accepts any non-revoked Bearer token -> 200.
    // Conversely, an opaque external token sent to a jwt endpoint fails
    // local signature validation -> 401.
    let jwt_cfg = JwtConfig {
        secret: "oauth2-test-jwt-secret-32chars!".to_string(),
        algorithm: main_serve::config::types::JwtAlgorithm::HS256,
        issuer: "main-serve".to_string(),
        audience: "my-app".to_string(),
        expiry: 3600,
        role_claim: "role".to_string(),
    };
    let jwt = create_token("local-user", Some("user"), &jwt_cfg).unwrap();

    // JWT on oauth2 endpoint -> forwarded to /userinfo as Bearer -> mock accepts -> 200.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/profile")
                .header("authorization", format!("Bearer {jwt}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Opaque external token on JWT endpoint -> fails local JWT validation -> 401.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .header("authorization", "Bearer not-a-valid-jwt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// =============================================================================
// Scenario 6: Reverse Proxy - Forwarding, Path Rewrite, Custom Headers
// =============================================================================

/// Start a mock upstream HTTP server that echoes the request path, method,
/// query string, and selected headers back as JSON.
///
/// Returns (base_url, shutdown_sender).
async fn start_mock_upstream() -> (String, tokio::sync::oneshot::Sender<()>) {
    use axum::Router;
    use axum::routing::any;
    use std::net::SocketAddr;
    use tokio::net::TcpListener;

    let app = Router::new().fallback(any(
        |method: axum::http::Method,
         uri: axum::http::Uri,
         headers: axum::http::HeaderMap,
         body: Body| async move {
            let body_bytes = body
                .collect()
                .await
                .map(|c| c.to_bytes())
                .unwrap_or_default();

            let mut hdr_map = serde_json::Map::new();
            for (name, value) in headers.iter() {
                if let Ok(v) = value.to_str() {
                    hdr_map.insert(name.to_string(), serde_json::Value::String(v.to_string()));
                }
            }

            axum::Json(serde_json::json!({
                "method": method.as_str(),
                "path": uri.path(),
                "query": uri.query().unwrap_or(""),
                "headers": hdr_map,
                "body": String::from_utf8_lossy(&body_bytes),
            }))
        },
    ));

    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind mock upstream");
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

/// Set up a server with the proxy config, replacing __UPSTREAM_URL__.
async fn setup_proxy_server(upstream_url: &str) -> (axum::Router, tempfile::NamedTempFile) {
    let yaml = PROXY_YAML.replace("__UPSTREAM_URL__", upstream_url);
    setup_server(&yaml).await
}

#[tokio::test]
async fn test_proxy_api_gateway_forwards_and_rewrites_path() {
    let (upstream_url, _shutdown) = start_mock_upstream().await;
    let (app, _f) = setup_proxy_server(&upstream_url).await;

    // GET /api/v1/users -> upstream /v1/users
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/users")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = json_body(resp).await;
    assert_eq!(json["path"], "/v1/users");
    assert_eq!(json["method"], "GET");
    // Custom headers injected
    assert_eq!(json["headers"]["x-forwarded-by"], "main-serve");
    assert_eq!(json["headers"]["x-request-source"], "gateway");
}

#[tokio::test]
async fn test_proxy_api_gateway_preserves_query_string() {
    let (upstream_url, _shutdown) = start_mock_upstream().await;
    let (app, _f) = setup_proxy_server(&upstream_url).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/items?active=true&page=2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = json_body(resp).await;
    assert_eq!(json["path"], "/v1/items");
    let query = json["query"].as_str().unwrap();
    assert!(query.contains("active=true"), "query: {query}");
    assert!(query.contains("page=2"), "query: {query}");
}

#[tokio::test]
async fn test_proxy_api_gateway_forwards_post_body() {
    let (upstream_url, _shutdown) = start_mock_upstream().await;
    let (app, _f) = setup_proxy_server(&upstream_url).await;

    let payload = r#"{"name":"new item"}"#;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/items")
                .header("content-type", "application/json")
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = json_body(resp).await;
    assert_eq!(json["method"], "POST");
    assert_eq!(json["path"], "/v1/items");
    assert!(json["body"].as_str().unwrap().contains("new item"));
}

#[tokio::test]
async fn test_proxy_legacy_path_rewrite() {
    let (upstream_url, _shutdown) = start_mock_upstream().await;
    let (app, _f) = setup_proxy_server(&upstream_url).await;

    // GET /legacy/widgets -> upstream /api/v2/widgets
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/legacy/widgets")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = json_body(resp).await;
    assert_eq!(json["path"], "/api/v2/widgets");
}

#[tokio::test]
async fn test_proxy_external_no_rewrite() {
    let (upstream_url, _shutdown) = start_mock_upstream().await;
    let (app, _f) = setup_proxy_server(&upstream_url).await;

    // GET /external/time -> upstream /external/time (no rewrite)
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/external/time")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = json_body(resp).await;
    assert_eq!(json["path"], "/external/time");
}

// =============================================================================
// Scenario 7: CRUD API - Joins, Computed Fields, Filtering, Where Clause
// =============================================================================

#[tokio::test]
async fn test_crud_api_author_lifecycle() {
    let yaml = &*CRUD_API_YAML;

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_crud_api_author");
        let (app, _state, _pool) = test_db.setup_app(yaml, "crud_api.yaml").await;

        // Create author (public endpoint)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/authors")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name": "Alice", "email": "alice@example.com", "bio": "Writer"})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "backend: {backend}");

        // List authors (public)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/authors")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");
        let list = json_body(resp).await;
        let data = list["data"].as_array().unwrap();
        assert_eq!(data.len(), 1, "backend: {backend}");
        assert_eq!(data[0]["name"], "Alice");

        // Update author requires auth
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/authors/1")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name": "Alice Updated"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "update without auth, backend: {backend}"
        );

        // Update with writer key
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/authors/1")
                    .header("X-API-Key", "write-key-002")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name": "Alice Updated"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");
    }
}

#[tokio::test]
async fn test_crud_api_articles_with_joins_and_computed_fields() {
    let yaml = &*CRUD_API_YAML;

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_crud_api_joins");
        let (app, _state, _pool) = test_db.setup_app(yaml, "crud_api.yaml").await;

        // Seed: create an author
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/authors")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name": "Bob", "email": "bob@example.com"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "backend: {backend}");

        // Seed: create a category
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/categories")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name": "Tech", "description": "Technology articles"})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "backend: {backend}");

        // Create a published article (requires writer key)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/articles")
                    .header("X-API-Key", "write-key-002")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "title": "Rust is Great",
                            "slug": "rust-is-great",
                            "body": "A detailed article about Rust programming.",
                            "status": "published",
                            "author_id": 1,
                            "category_id": 1
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "backend: {backend}");

        // Create a draft article (should NOT appear in public listing)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/articles")
                    .header("X-API-Key", "write-key-002")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "title": "Draft Post",
                            "slug": "draft-post",
                            "body": "Not ready yet.",
                            "status": "draft",
                            "author_id": 1
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "backend: {backend}");

        // List articles (public) - should only show the published one
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/articles")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");
        let list = json_body(resp).await;
        let data = list["data"].as_array().unwrap();
        assert_eq!(
            data.len(),
            1,
            "where_clause should filter drafts, backend: {backend}"
        );
        assert_eq!(data[0]["title"], "Rust is Great");
        // Joined field from authors table
        assert_eq!(
            data[0]["author_name"], "Bob",
            "INNER JOIN author_name missing, backend: {backend}"
        );
        // Joined field from categories table (LEFT JOIN)
        assert_eq!(
            data[0]["category_name"], "Tech",
            "LEFT JOIN category_name missing, backend: {backend}"
        );
        // Computed field
        let body_length = data[0]["body_length"].as_i64().unwrap();
        assert_eq!(
            body_length,
            "A detailed article about Rust programming.".len() as i64,
            "computed body_length, backend: {backend}"
        );
    }
}

#[tokio::test]
async fn test_crud_api_article_without_category_left_join() {
    let yaml = &*CRUD_API_YAML;

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_crud_api_left");
        let (app, _state, _pool) = test_db.setup_app(yaml, "crud_api.yaml").await;

        // Seed author
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/authors")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name": "Carol", "email": "carol@example.com"})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "backend: {backend}");

        // Create article without a category (category_id = null)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/articles")
                    .header("X-API-Key", "write-key-002")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "title": "No Category",
                            "slug": "no-category",
                            "body": "An article without a category.",
                            "status": "published",
                            "author_id": 1
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "backend: {backend}");

        // List - article should appear even though category_id is null (LEFT JOIN)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/articles")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "backend: {backend}");
        let list = json_body(resp).await;
        let data = list["data"].as_array().unwrap();
        assert_eq!(data.len(), 1, "backend: {backend}");
        assert_eq!(data[0]["author_name"], "Carol");
        assert!(
            data[0]["category_name"].is_null(),
            "LEFT JOIN with no match should return null, backend: {backend}, got: {:?}",
            data[0]["category_name"]
        );
    }
}

#[tokio::test]
async fn test_crud_api_rbac_admin_only_delete() {
    let yaml = &*CRUD_API_YAML;

    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "rw_crud_api_rbac");
        let (app, _state, _pool) = test_db.setup_app(yaml, "crud_api.yaml").await;

        // Seed author + article
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/authors")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name": "Dan", "email": "dan@example.com"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/articles")
                    .header("X-API-Key", "write-key-002")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "title": "To Delete",
                            "slug": "to-delete",
                            "body": "Temporary.",
                            "status": "published",
                            "author_id": 1
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Writer key should not be able to delete (admin only)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/articles/1")
                    .header("X-API-Key", "write-key-002")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "writer should not delete, backend: {backend}"
        );

        // Admin key can delete
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/articles/1")
                    .header("X-API-Key", "admin-key-003")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "admin should delete, backend: {backend}"
        );
    }
}

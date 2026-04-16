mod support;

// Hot reload integration tests.
//
// These tests spin up the server using the test helper and verify:
// - Health endpoint works
// - Reload endpoint requires auth
// - Reload endpoint accepts valid auth and reloads config
// - Reload rejects invalid config files
// - Custom response endpoints work

use std::io::Write;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use main_serve::config::load_config;
use main_serve::db::migration::run_migrations;
use main_serve::db::pool::create_pools;
use main_serve::server::{AppState, build_router};
use support::db::{TestDatabase, enabled_backends};
use support::setup_server;

async fn setup_server_with_db(
    test_db: &TestDatabase,
    yaml: &str,
    file_name: &str,
) -> (axum::Router, AppState, std::path::PathBuf) {
    let config_path = test_db.write_config(yaml, file_name);
    let config = load_config(&config_path).expect("load config");
    let pools = create_pools(&config.databases).await.expect("create pools");
    run_migrations(&config.tables, &pools, &config.databases)
        .await
        .expect("run migrations");

    let state = AppState::new(config, config_path.clone(), "test-token".to_string());
    {
        let mut pool_lock = state.db_pools.write().await;
        *pool_lock = pools;
    }
    let config_guard = state.config.read().await;
    let app = build_router(&config_guard, state.clone());
    drop(config_guard);
    (app, state, config_path)
}

const MINIMAL_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

endpoints:
  - path: "/api/info"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"test": true}'
    auth: "none"
"#;

const RELOAD_DB_V1: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: false

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

endpoints:
  - path: "/api/items"
    methods: ["get"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title"]
    auth: "none"
"#;

const RELOAD_DB_V2_ADD: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: false

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
      - name: "status"
        type: "varchar"
        nullable: false
        default: "'draft'"

endpoints:
  - path: "/api/items"
    methods: ["get"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "status"]
    auth: "none"
"#;

const RELOAD_DB_V1_DROP: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: true

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

endpoints:
  - path: "/api/items"
    methods: ["get"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "body"]
    auth: "none"
"#;

const RELOAD_DB_V2_DROP: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: true

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

endpoints:
  - path: "/api/items"
    methods: ["get"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title"]
    auth: "none"
"#;

#[tokio::test]
async fn test_health_endpoint() {
    let (app, _f) = setup_server(MINIMAL_CONFIG).await;

    let req = Request::builder()
        .uri("/_main-serve/health")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "healthy");
    assert_eq!(json["endpoints_configured"], 1);
}

#[tokio::test]
async fn test_reload_requires_auth() {
    let (app, _f) = setup_server(MINIMAL_CONFIG).await;

    let req = Request::builder()
        .method("POST")
        .uri("/_main-serve/reload")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_reload_rejects_wrong_token() {
    let (app, _f) = setup_server(MINIMAL_CONFIG).await;

    let req = Request::builder()
        .method("POST")
        .uri("/_main-serve/reload")
        .header("Authorization", "Bearer wrong-token")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_reload_with_valid_token() {
    let (app, mut f) = setup_server(MINIMAL_CONFIG).await;

    // Overwrite the temp file with a config that has 2 endpoints (was 1).
    f.as_file_mut().set_len(0).unwrap();
    std::io::Seek::seek(f.as_file_mut(), std::io::SeekFrom::Start(0)).unwrap();
    f.write_all(
        br#"
server:
  host: "127.0.0.1"
  port: 0

endpoints:
  - path: "/api/info"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"test": true}'
    auth: "none"
  - path: "/api/extra"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"extra": true}'
    auth: "none"
"#,
    )
    .unwrap();
    f.flush().unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/_main-serve/reload")
        .header("Authorization", "Bearer test-token")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "reloaded");
    // Verify the reload summary reflects the new config (2 endpoints, not 1).
    assert_eq!(
        json["summary"]["endpoints"], 2,
        "Reloaded config should have 2 endpoints"
    );

    // Verify via health check that the in-memory state was actually updated.
    let health_req = Request::builder()
        .uri("/_main-serve/health")
        .body(Body::empty())
        .unwrap();
    let health_resp = app.oneshot(health_req).await.unwrap();
    let health_body = health_resp.into_body().collect().await.unwrap().to_bytes();
    let health_json: serde_json::Value = serde_json::from_slice(&health_body).unwrap();
    assert_eq!(
        health_json["endpoints_configured"], 2,
        "Health check should report 2 endpoints after reload"
    );
}

#[tokio::test]
async fn test_reload_rejects_invalid_config() {
    // Start with a valid config, then overwrite the temp file with invalid YAML.
    let (app, mut f) = setup_server(MINIMAL_CONFIG).await;

    // Overwrite config file with invalid content.
    f.as_file_mut().set_len(0).unwrap();
    std::io::Seek::seek(f.as_file_mut(), std::io::SeekFrom::Start(0)).unwrap();
    f.write_all(b"endpoints:\n  - path: '/bad'\n    methods: ['get']\n    action: 'crud'\n    auth: 'none'\n").unwrap();
    f.flush().unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/_main-serve/reload")
        .header("Authorization", "Bearer test-token")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    // Should fail validation (crud action without crud config).
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_custom_response_endpoint() {
    let (app, _f) = setup_server(MINIMAL_CONFIG).await;

    let req = Request::builder()
        .uri("/api/info")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let ct = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("application/json"));

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["test"], true);
}

#[tokio::test]
async fn test_custom_response_with_custom_headers() {
    let yaml = r#"
server:
  port: 0
endpoints:
  - path: "/headers"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 201
      content_type: "text/plain"
      body: "created"
      headers:
        X-Custom-Header: "custom-value"
    auth: "none"
"#;
    let (app, _f) = setup_server(yaml).await;

    let req = Request::builder()
        .uri("/headers")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response
            .headers()
            .get("x-custom-header")
            .unwrap()
            .to_str()
            .unwrap(),
        "custom-value"
    );
}

#[tokio::test]
async fn test_crud_needs_db_pool() {
    // CRUD endpoints without DB pools return an internal error since no pool exists.
    let yaml = r#"
server:
  port: 0
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"
tables:
  items:
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true
endpoints:
  - path: "/api/items"
    methods: ["get"]
    action: "crud"
    crud:
      table: "items"
      database: "main"
    auth: "none"
"#;
    let (app, _f) = setup_server(yaml).await;

    let req = Request::builder()
        .uri("/api/items")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    // Without a real DB pool in state, the handler returns 500.
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn test_nonexistent_route_returns_404() {
    let (app, _f) = setup_server(MINIMAL_CONFIG).await;

    let req = Request::builder()
        .uri("/does-not-exist")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_reload_adds_column_on_db_backed_config_across_backends() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "reload_add_column");
        let (app, state, config_path) =
            setup_server_with_db(&test_db, RELOAD_DB_V1, "reload_v1.yaml").await;

        {
            let pools = state.db_pools.read().await;
            let pool = pools.get("main").unwrap();
            let insert_sql = format!(
                "INSERT INTO {} (title) VALUES ({})",
                test_db.table_name,
                backend.placeholder(1)
            );
            pool.execute_with_params(&insert_sql, &[serde_json::json!("hello")])
                .await
                .expect("insert row before reload");
        }

        std::fs::write(&config_path, test_db.render_yaml(RELOAD_DB_V2_ADD)).unwrap();

        let req = Request::builder()
            .method("POST")
            .uri("/_main-serve/reload")
            .header("Authorization", "Bearer test-token")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");

        let pools = state.db_pools.read().await;
        let select_sql = format!(
            "SELECT title, status FROM {} ORDER BY id",
            test_db.table_name
        );
        let rows = pools
            .get("main")
            .unwrap()
            .fetch_all_json(&select_sql, &[])
            .await
            .expect("select rows after reload");
        assert_eq!(rows[0]["title"], "hello", "backend: {backend}");
        assert_eq!(rows[0]["status"], "draft", "backend: {backend}");
    }
}

#[tokio::test]
async fn test_reload_drops_column_with_allow_destructive_across_backends() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "reload_drop_column");
        let (app, state, config_path) =
            setup_server_with_db(&test_db, RELOAD_DB_V1_DROP, "reload_drop_v1.yaml").await;

        std::fs::write(&config_path, test_db.render_yaml(RELOAD_DB_V2_DROP)).unwrap();

        let req = Request::builder()
            .method("POST")
            .uri("/_main-serve/reload")
            .header("Authorization", "Bearer test-token")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");

        let pools = state.db_pools.read().await;
        let select_sql = format!("SELECT body FROM {} LIMIT 1", test_db.table_name);
        let select_result = pools
            .get("main")
            .unwrap()
            .fetch_all_json(&select_sql, &[])
            .await;
        assert!(select_result.is_err(), "backend: {backend}");
    }
}

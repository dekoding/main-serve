/// Config parsing and validation integration tests.
mod support;
use crate::support::helpers::load_yaml;

#[test]
fn test_minimal_config_loads() {
    let yaml = r#"
server:
  host: "127.0.0.1"
  port: 3000
"#;
    let config = load_yaml(yaml).unwrap();
    assert_eq!(config.server.host, "127.0.0.1");
    assert_eq!(config.server.port, 3000);
}

#[test]
fn test_defaults_applied() {
    let yaml = "{}";
    let config = load_yaml(yaml).unwrap();
    assert_eq!(config.server.host, "127.0.0.1");
    assert_eq!(config.server.port, 8080);
    assert_eq!(config.server.workers, 0);
    assert_eq!(config.server.max_body_size, 10 * 1024 * 1024);
    assert_eq!(config.server.keep_alive, 75);
    assert_eq!(config.server.shutdown_timeout, 30);
    assert!(config.server.tls.is_none());
    assert_eq!(
        config.logging.level,
        main_serve::config::types::LogLevel::Info
    );
    assert_eq!(
        config.logging.format,
        main_serve::config::types::LogFormat::Pretty
    );
    assert!(!config.logging.log_request_body);
    assert!(!config.logging.log_response_body);
    assert!(config.databases.is_empty());
    assert!(config.endpoints.is_empty());
}

#[test]
fn test_database_config_parses() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"
    max_connections: 5
"#;
    let config = load_yaml(yaml).unwrap();
    let db = config.databases.get("main").unwrap();
    assert_eq!(db.driver, main_serve::config::types::DatabaseDriver::Sqlite);
    assert_eq!(db.url, "sqlite://test.db");
    assert_eq!(db.max_connections, 5);
    assert_eq!(db.min_connections, 1); // default
}

#[test]
fn test_table_config_parses() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"
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
"#;
    let config = load_yaml(yaml).unwrap();
    let table = config.tables.iter().find(|t| t.name == "users").unwrap();
    assert_eq!(table.columns.len(), 2);
    assert_eq!(table.columns[0].name, "id");
    assert!(table.columns[0].primary_key);
    assert!(table.columns[1].unique);
}

#[test]
fn test_endpoint_crud_parses() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"
tables:
  - name: "posts"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true
      - name: "title"
        type: "text"
endpoints:
  - path: "/api/posts"
    methods: ["get", "post"]
    action: "crud"
    crud:
      table: "posts"
      database: "main"
      fields: ["id", "title"]
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    assert_eq!(config.endpoints.len(), 1);
    assert_eq!(config.endpoints[0].path, "/api/posts");
    assert_eq!(
        config.endpoints[0].action,
        main_serve::config::types::EndpointAction::Crud
    );
    let crud = config.endpoints[0].crud.as_ref().unwrap();
    assert_eq!(crud.table, "posts");
    assert_eq!(crud.fields, vec!["id", "title"]);
}

#[test]
fn test_endpoint_custom_response_parses() {
    let yaml = r#"
endpoints:
  - path: "/api/info"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"ok": true}'
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let cr = config.endpoints[0].custom_response.as_ref().unwrap();
    assert_eq!(cr.status, 200);
    assert_eq!(cr.content_type, "application/json");
}

#[test]
fn test_auth_jwt_parses() {
    let yaml = r#"
auth:
  jwt:
    secret: "test-secret-key-12345"
    algorithm: "HS256"
    issuer: "test"
    expiry: 7200
"#;
    let config = load_yaml(yaml).unwrap();
    let jwt = config.auth.jwt.as_ref().unwrap();
    assert_eq!(jwt.secret, "test-secret-key-12345");
    assert_eq!(jwt.issuer, "test");
    assert_eq!(jwt.expiry, 7200);
}

#[test]
fn test_auth_api_key_parses() {
    let yaml = r#"
auth:
  api_key:
    location: "header"
    name: "X-API-Key"
    keys:
      - key: "key-1"
        role: "admin"
      - key: "key-2"
"#;
    let config = load_yaml(yaml).unwrap();
    let ak = config.auth.api_key.as_ref().unwrap();
    assert_eq!(ak.keys.len(), 2);
    assert_eq!(ak.keys[0].role.as_deref(), Some("admin"));
    assert!(ak.keys[1].role.is_none());
}

#[test]
fn test_env_var_interpolation() {
    // SAFETY: test-only env var manipulation, serialized by test framework
    unsafe { std::env::set_var("TEST_DB_URL", "postgres://localhost/testdb") };
    let yaml = r#"
databases:
  main:
    driver: "postgres"
    url: "${TEST_DB_URL}"
"#;
    let config = load_yaml(yaml).unwrap();
    assert_eq!(config.databases["main"].url, "postgres://localhost/testdb");
    // SAFETY: test-only, single-threaded test runner
    unsafe { std::env::remove_var("TEST_DB_URL") };
}

#[test]
fn test_env_var_with_default() {
    // SAFETY: test-only, single-threaded test runner
    unsafe { std::env::remove_var("TEST_NONEXIST_PORT") };
    let yaml = r"
server:
  port: ${TEST_NONEXIST_PORT:-9090}
";
    let config = load_yaml(yaml).unwrap();
    assert_eq!(config.server.port, 9090);
}

#[test]
fn test_validation_missing_table_ref() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"
endpoints:
  - path: "/api/posts"
    methods: ["get"]
    action: "crud"
    crud:
      table: "nonexistent"
      database: "main"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("nonexistent"),
        "Error should mention the missing table: {err}"
    );
}

#[test]
fn test_validation_missing_db_ref() {
    let yaml = r#"
tables:
  - name: "posts"
    database: "nonexistent"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("nonexistent"),
        "Error should mention the missing database: {err}"
    );
}

#[test]
fn test_validation_table_no_pk() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"
tables:
  - name: "posts"
    database: "main"
    columns:
      - name: "title"
        type: "text"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("primary key"),
        "Error should mention missing PK: {err}"
    );
}

#[test]
fn test_validation_crud_without_config() {
    let yaml = r#"
endpoints:
  - path: "/api/test"
    methods: ["get"]
    action: "crud"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("crud"),
        "Error should mention missing crud config: {err}"
    );
}

#[test]
fn test_validation_auth_ref_invalid() {
    let yaml = r#"
endpoints:
  - path: "/api/test"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: "ok"
    auth: "jwt"
"#;
    // jwt auth referenced but not configured
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("jwt"),
        "Error should mention invalid auth ref: {err}"
    );
}

#[test]
fn test_cors_config_parses() {
    let yaml = r#"
cors:
  allowed_origins: ["https://example.com"]
  allowed_methods: ["GET", "POST"]
  allow_credentials: true
  max_age: 3600
"#;
    let config = load_yaml(yaml).unwrap();
    assert_eq!(config.cors.allowed_origins, vec!["https://example.com"]);
    assert!(config.cors.allow_credentials);
    assert_eq!(config.cors.max_age, 3600);
}

#[test]
fn test_rate_limit_config_parses() {
    let yaml = r#"
rate_limit:
  enabled: true
  max_requests: 50
  window_seconds: 30
  key_strategy: "header"
  key_header: "X-Client-Id"
"#;
    let config = load_yaml(yaml).unwrap();
    assert!(config.rate_limit.enabled);
    assert_eq!(config.rate_limit.max_requests, 50);
    assert_eq!(
        config.rate_limit.key_strategy,
        main_serve::config::types::RateLimitKeyStrategy::Header
    );
}

#[test]
fn test_demo_config_loads() {
    let path = std::path::Path::new("config/templates/default.yaml");
    let config = main_serve::config::load_config(path).unwrap();
    assert_eq!(config.server.port, 8080);
    assert!(!config.endpoints.is_empty());
}

#[test]
fn test_logging_config_parses() {
    let yaml = r#"
logging:
  level: "debug"
  format: "json"
  log_request_body: true
  log_response_body: true
"#;
    let config = load_yaml(yaml).unwrap();
    assert_eq!(
        config.logging.level,
        main_serve::config::types::LogLevel::Debug
    );
    assert_eq!(
        config.logging.format,
        main_serve::config::types::LogFormat::Json
    );
    assert!(config.logging.log_request_body);
    assert!(config.logging.log_response_body);
}

#[test]
fn test_server_workers_parses() {
    let yaml = r"
server:
  workers: 4
  keep_alive: 120
";
    let config = load_yaml(yaml).unwrap();
    assert_eq!(config.server.workers, 4);
    assert_eq!(config.server.keep_alive, 120);
}

#[test]
fn test_per_endpoint_cors_parses() {
    let yaml = r#"
endpoints:
  - path: "/api/data"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: "ok"
    cors:
      allowed_origins: ["https://special.example.com"]
      allowed_methods: ["GET"]
      allow_credentials: true
      max_age: 600
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let ep = &config.endpoints[0];
    let cors = ep.cors.as_ref().expect("per-endpoint cors should be Some");
    assert_eq!(cors.allowed_origins, vec!["https://special.example.com"]);
    assert_eq!(cors.allowed_methods, vec!["GET"]);
    assert!(cors.allow_credentials);
    assert_eq!(cors.max_age, 600);
}

// =========================================================================
// $include directive tests
// =========================================================================

#[test]
fn test_include_single_endpoint_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");

    // Write an endpoint definition to a separate file.
    let endpoint_yaml = r#"
path: "/api/health"
methods: ["get"]
action: custom_response
custom_response:
  status: 200
  body: "ok"
auth: none
"#;
    std::fs::create_dir_all(dir.path().join("endpoints")).unwrap();
    std::fs::write(dir.path().join("endpoints/health.yaml"), endpoint_yaml).unwrap();

    // Main config includes it.
    let main_yaml = r#"
server:
  port: 0

endpoints:
  - $include: "endpoints/health.yaml"
"#;
    std::fs::write(dir.path().join("config.yaml"), main_yaml).unwrap();

    let config = main_serve::config::load_config(&dir.path().join("config.yaml")).unwrap();
    assert_eq!(config.endpoints.len(), 1);
    assert_eq!(config.endpoints[0].path, "/api/health");
}

#[test]
fn test_include_glob_multiple_endpoints() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("endpoints")).unwrap();

    // Endpoint A
    std::fs::write(
        dir.path().join("endpoints/a_users.yaml"),
        r#"
path: "/api/users"
methods: ["get"]
action: custom_response
custom_response:
  status: 200
  body: "users"
auth: none
"#,
    )
    .unwrap();

    // Endpoint B
    std::fs::write(
        dir.path().join("endpoints/b_items.yaml"),
        r#"
path: "/api/items"
methods: ["get"]
action: custom_response
custom_response:
  status: 200
  body: "items"
auth: none
"#,
    )
    .unwrap();

    let main_yaml = r#"
server:
  port: 0

endpoints:
  - $include: "endpoints/*.yaml"
"#;
    std::fs::write(dir.path().join("config.yaml"), main_yaml).unwrap();

    let config = main_serve::config::load_config(&dir.path().join("config.yaml")).unwrap();
    assert_eq!(config.endpoints.len(), 2);
    // Glob results are sorted, so a_users comes before b_items.
    assert_eq!(config.endpoints[0].path, "/api/users");
    assert_eq!(config.endpoints[1].path, "/api/items");
}

#[test]
fn test_include_mixed_inline_and_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("endpoints")).unwrap();

    std::fs::write(
        dir.path().join("endpoints/external.yaml"),
        r#"
path: "/api/ext"
methods: ["get"]
action: custom_response
custom_response:
  status: 200
  body: "external"
auth: none
"#,
    )
    .unwrap();

    let main_yaml = r#"
server:
  port: 0

endpoints:
  - path: "/inline"
    methods: ["get"]
    action: custom_response
    custom_response:
      status: 200
      body: "inline"
    auth: none
  - $include: "endpoints/external.yaml"
"#;
    std::fs::write(dir.path().join("config.yaml"), main_yaml).unwrap();

    let config = main_serve::config::load_config(&dir.path().join("config.yaml")).unwrap();
    assert_eq!(config.endpoints.len(), 2);
    assert_eq!(config.endpoints[0].path, "/inline");
    assert_eq!(config.endpoints[1].path, "/api/ext");
}

#[test]
fn test_include_mapping_level() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("db")).unwrap();

    // A file that produces a mapping with one database entry.
    std::fs::write(
        dir.path().join("db/databases.yaml"),
        r#"
main:
  driver: sqlite
  url: "sqlite://test.db?mode=rwc"
"#,
    )
    .unwrap();

    let main_yaml = r#"
server:
  port: 0

databases:
  $include: "db/databases.yaml"
"#;
    std::fs::write(dir.path().join("config.yaml"), main_yaml).unwrap();

    let config = main_serve::config::load_config(&dir.path().join("config.yaml")).unwrap();
    assert!(config.databases.contains_key("main"));
    assert_eq!(config.databases["main"].url, "sqlite://test.db?mode=rwc");
}

#[test]
fn test_include_circular_detection() {
    let dir = tempfile::TempDir::new().expect("tempdir");

    // File A includes file B, which includes file A.
    std::fs::write(
        dir.path().join("a.yaml"),
        r#"
server:
  port: 0
endpoints:
  - $include: "b.yaml"
"#,
    )
    .unwrap();

    std::fs::write(
        dir.path().join("b.yaml"),
        r#"
path: "/x"
methods: ["get"]
action: custom_response
custom_response:
  status: 200
  body: "x"
auth: none
endpoints:
  - $include: "a.yaml"
"#,
    )
    .unwrap();

    let result = main_serve::config::load_config(&dir.path().join("a.yaml"));
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("Circular"),
        "Expected circular include error, got: {err}"
    );
}

#[test]
fn test_include_no_match_is_error() {
    let dir = tempfile::TempDir::new().expect("tempdir");

    let main_yaml = r#"
server:
  port: 0
endpoints:
  - $include: "nonexistent/*.yaml"
"#;
    std::fs::write(dir.path().join("config.yaml"), main_yaml).unwrap();

    let result = main_serve::config::load_config(&dir.path().join("config.yaml"));
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("matched no files"),
        "Expected no-match error, got: {err}"
    );
}

#[test]
fn test_include_env_vars_in_included_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("endpoints")).unwrap();

    // The included file uses env var interpolation.
    std::fs::write(
        dir.path().join("endpoints/api.yaml"),
        r#"
path: "/api/test"
methods: ["get"]
action: custom_response
custom_response:
  status: 200
  body: "${TEST_INCLUDE_BODY:-hello_from_include}"
auth: none
"#,
    )
    .unwrap();

    let main_yaml = r#"
server:
  port: 0
endpoints:
  - $include: "endpoints/api.yaml"
"#;
    std::fs::write(dir.path().join("config.yaml"), main_yaml).unwrap();

    let config = main_serve::config::load_config(&dir.path().join("config.yaml")).unwrap();
    let cr = config.endpoints[0].custom_response.as_ref().unwrap();
    assert_eq!(cr.body, "hello_from_include");
}

#[test]
fn test_include_nested() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("parts")).unwrap();

    // The included file itself includes another file.
    std::fs::write(
        dir.path().join("parts/endpoints.yaml"),
        r#"
- $include: "../parts/ping.yaml"
"#,
    )
    .unwrap();

    std::fs::write(
        dir.path().join("parts/ping.yaml"),
        r#"
path: "/ping"
methods: ["get"]
action: custom_response
custom_response:
  status: 200
  body: "pong"
auth: none
"#,
    )
    .unwrap();

    let main_yaml = r#"
server:
  port: 0
endpoints:
  $include: "parts/endpoints.yaml"
"#;
    std::fs::write(dir.path().join("config.yaml"), main_yaml).unwrap();

    let config = main_serve::config::load_config(&dir.path().join("config.yaml")).unwrap();
    assert_eq!(config.endpoints.len(), 1);
    assert_eq!(config.endpoints[0].path, "/ping");
}

// =========================================================================
// Store validation tests
// =========================================================================

#[test]
fn test_store_native_with_root_parses() {
    let yaml = r#"
stores:
  assets:
    backend: native
    root: "./public"
"#;
    let config = load_yaml(yaml).unwrap();
    let store = config.stores.get("assets").unwrap();
    assert_eq!(
        store.backend,
        main_serve::config::types::StoreBackend::Native
    );
    assert_eq!(store.root.as_deref(), Some("./public"));
}

#[test]
fn test_store_s3_parses() {
    let yaml = r#"
stores:
  s3_assets:
    backend: s3
    s3:
      region: "us-east-1"
      bucket: "my-bucket"
      access_key: "AKIAIOSFODNN7EXAMPLE"
      secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
      endpoint: "https://minio.example.com"
      force_path_style: true
"#;
    let config = load_yaml(yaml).unwrap();
    let store = config.stores.get("s3_assets").unwrap();
    assert_eq!(store.backend, main_serve::config::types::StoreBackend::S3);
    let s3 = store.s3.as_ref().unwrap();
    assert_eq!(s3.region, "us-east-1");
    assert_eq!(s3.bucket, "my-bucket");
}

#[test]
fn test_store_azure_parses() {
    let yaml = r#"
stores:
  azure_assets:
    backend: azure
    azure:
      account_name: "myaccount"
      account_key: "mykey123"
      container: "assets"
"#;
    let config = load_yaml(yaml).unwrap();
    let store = config.stores.get("azure_assets").unwrap();
    assert_eq!(
        store.backend,
        main_serve::config::types::StoreBackend::Azure
    );
    let azure = store.azure.as_ref().unwrap();
    assert_eq!(azure.account_name, "myaccount");
    assert_eq!(azure.container, "assets");
}

#[test]
fn test_store_gcs_parses() {
    let yaml = r#"
stores:
  gcs_assets:
    backend: gcs
    gcs:
      project_id: "my-project"
      credentials: '{"type":"service_account"}'
      bucket: "my-gcs-bucket"
"#;
    let config = load_yaml(yaml).unwrap();
    let store = config.stores.get("gcs_assets").unwrap();
    assert_eq!(store.backend, main_serve::config::types::StoreBackend::Gcs);
    let gcs = store.gcs.as_ref().unwrap();
    assert_eq!(gcs.project_id, "my-project");
    assert_eq!(gcs.bucket, "my-gcs-bucket");
}

#[test]
fn test_validation_native_store_missing_root() {
    let yaml = r"
stores:
  empty_native:
    backend: native
";
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("root directory"),
        "Error should mention missing root for native: {err}"
    );
}

#[test]
fn test_validation_s3_store_missing_config() {
    let yaml = r"
stores:
  broken_s3:
    backend: s3
";
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("s3 config section"),
        "Error should mention missing s3 config: {err}"
    );
}

#[test]
fn test_validation_s3_store_missing_region() {
    let yaml = r#"
stores:
  broken_s3:
    backend: s3
    s3:
      bucket: "my-bucket"
      access_key: "key"
      secret_key: "secret"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("s3.region"),
        "Error should mention missing region: {err}"
    );
}

#[test]
fn test_validation_azure_store_missing_fields() {
    let yaml = r#"
stores:
  broken_azure:
    backend: azure
    azure:
      account_name: "myaccount"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("azure.container"),
        "Error should mention missing container: {err}"
    );
}

#[test]
fn test_validation_gcs_store_missing_fields() {
    let yaml = r#"
stores:
  broken_gcs:
    backend: gcs
    gcs:
      project_id: "my-project"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("gcs.bucket"),
        "Error should mention missing bucket: {err}"
    );
}

#[test]
fn test_validation_store_conflicting_backend_configs() {
    let yaml = r#"
stores:
  conflicting:
    backend: native
    root: "./data"
    s3:
      region: "us-east-1"
      bucket: "bucket"
      access_key: "key"
      secret_key: "secret"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("conflicting"),
        "Error should mention conflicting configs: {err}"
    );
}

// =========================================================================
// Endpoint store reference validation tests
// =========================================================================

#[test]
fn test_validation_staticfiles_missing_store_ref() {
    let yaml = r#"
stores:
  assets:
    backend: native
    root: "./public"
endpoints:
  - path: "/assets"
    methods: ["get"]
    action: "static_files"
    static_files:
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
fn test_validation_staticfiles_empty_storage() {
    let yaml = r#"
stores:
  assets:
    backend: native
    root: "./public"
endpoints:
  - path: "/assets"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: ""
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("storage must not be empty"),
        "Error should mention empty storage: {err}"
    );
}

#[test]
fn test_validation_spa_host_missing_store_ref() {
    let yaml = r#"
endpoints:
  - path: "/app"
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
fn test_validation_media_missing_refs() {
    let yaml = r#"
endpoints:
  - path: "/media"
    methods: ["get"]
    action: "media"
    media:
      storage: "nonexistent"
      table: "media_items"
      database: "nonexistent_db"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("nonexistent"),
        "Error should mention missing references: {err}"
    );
}

#[test]
fn test_validation_filestore_missing_refs() {
    let yaml = r#"
endpoints:
  - path: "/files"
    methods: ["get"]
    action: "file_store"
    file_store:
      storage: "nonexistent"
      table: "file_registry"
      database: "nonexistent_db"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("nonexistent"),
        "Error should mention missing references: {err}"
    );
}

#[test]
fn test_store_with_env_var_interpolation() {
    // SAFETY: test-only, single-threaded test runner
    unsafe { std::env::set_var("TEST_STORE_ROOT", "/var/data/assets") };
    let yaml = r#"
stores:
  assets:
    backend: native
    root: "${TEST_STORE_ROOT}"
"#;
    let config = load_yaml(yaml).unwrap();
    let store = config.stores.get("assets").unwrap();
    assert_eq!(store.root.as_deref(), Some("/var/data/assets"));
    // SAFETY: test-only, single-threaded test runner
    unsafe { std::env::remove_var("TEST_STORE_ROOT") };
}

// =========================================================================
// 5.3 Config validation edge cases
// =========================================================================

#[test]
fn test_validation_duplicate_store_names_rejected() {
    // Per spec: "Duplicate store names are a configuration error (fail-fast at load time)"
    // serde_yaml rejects duplicate keys at parse time.
    let yaml = r#"
stores:
  assets:
    backend: native
    root: "./public1"
  assets:
    backend: native
    root: "./public2"
"#;
    // serde_yaml rejects duplicate keys
    let result = load_yaml(yaml);
    assert!(
        result.is_err(),
        "Duplicate store names should be rejected: {:?}",
        result
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("duplicate") || err.contains("Duplicate"),
        "Error should mention duplicate: {err}"
    );
}

#[test]
fn test_validation_invalid_http_method_rejected() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"

endpoints:
  - path: "/api/test"
    methods: ["get", "invalid_method"]
    action: "crud"
    crud:
      table: "posts"
      database: "main"
    auth: "none"
"#;
    // Invalid methods should be rejected at deserialization time
    let result = load_yaml(yaml);
    assert!(
        result.is_err(),
        "Invalid HTTP method should be rejected: {:?}",
        result
    );
}

#[test]
fn test_validation_endpoint_empty_path_rejected() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"

endpoints:
  - path: ""
    methods: ["get"]
    action: "crud"
    crud:
      table: "posts"
      database: "main"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(
        result.is_err(),
        "Empty endpoint path should be rejected: {:?}",
        result
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("path must not be empty") || err.contains("empty"),
        "Error should mention empty path: {err}"
    );
}

#[test]
fn test_validation_endpoint_no_methods_rejected() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"

endpoints:
  - path: "/api/test"
    methods: []
    action: "crud"
    crud:
      table: "posts"
      database: "main"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(
        result.is_err(),
        "Endpoint with no methods should be rejected: {:?}",
        result
    );
}

#[test]
fn test_validation_spa_host_default_index_used() {
    // spa_host config has a default index value ("index.html")
    // when omitted, so it should parse successfully with the default
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
    let spa = config.endpoints[0].spa_host.as_ref().unwrap();
    assert_eq!(spa.storage, "assets");
    // Default index should be "index.html"
    assert_eq!(spa.index, "index.html");
}

#[test]
fn test_validation_duplicate_table_same_database() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"

tables:
  - name: "users"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
  - name: "users"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
"#;
    let result = load_yaml(yaml);
    assert!(
        result.is_err(),
        "Duplicate table in same database should fail: {:?}",
        result
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Duplicate") || err.contains("duplicate"),
        "Error should mention duplicate: {err}"
    );
}

#[test]
fn test_validation_table_column_duplicate_names() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"

tables:
  - name: "users"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "id"
        type: "text"
"#;
    let result = load_yaml(yaml);
    assert!(
        result.is_err(),
        "Duplicate column names should fail: {:?}",
        result
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("duplicate"),
        "Error should mention duplicate column: {err}"
    );
}

#[test]
fn test_validation_custom_response_invalid_status() {
    let yaml = r#"
endpoints:
  - path: "/api/test"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 999
      body: "ok"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(
        result.is_err(),
        "Invalid status code (999) should be rejected: {:?}",
        result
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("status") && err.contains("valid"),
        "Error should mention invalid status: {err}"
    );
}

#[test]
fn test_validation_custom_response_valid_status() {
    let yaml = r#"
endpoints:
  - path: "/api/test"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 201
      body: "created"
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    assert_eq!(config.endpoints.len(), 1);
    let cr = config.endpoints[0].custom_response.as_ref().unwrap();
    assert_eq!(cr.status, 201);
}

#[test]
fn test_validation_file_store_valid_refs() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"

tables:
  - name: "file_registry"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true

stores:
  file_store:
    backend: native
    root: "./files"

endpoints:
  - path: "/files/*"
    methods: ["get"]
    action: "file_store"
    file_store:
      storage: "file_store"
      table: "file_registry"
      database: "main"
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    assert_eq!(config.endpoints.len(), 1);
}

#[test]
fn test_validation_media_valid_refs() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"

tables:
  - name: "media_items"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true

stores:
  media_store:
    backend: native
    root: "./media"

endpoints:
  - path: "/media/*"
    methods: ["get"]
    action: "media"
    media:
      storage: "media_store"
      table: "media_items"
      database: "main"
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    assert_eq!(config.endpoints.len(), 1);
}

#[test]
fn test_validation_proxy_missing_upstream() {
    let yaml = r#"
endpoints:
  - path: "/proxy/*"
    methods: ["get"]
    action: "proxy"
    proxy:
      upstream: ""
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(
        result.is_err(),
        "Empty upstream should be rejected: {:?}",
        result
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("upstream"),
        "Error should mention upstream: {err}"
    );
}

#[test]
fn test_validation_auth_provider_not_configured() {
    let yaml = r#"
endpoints:
  - path: "/api/test"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: "ok"
    auth: "nonexistent_provider"
"#;
    let result = load_yaml(yaml);
    assert!(
        result.is_err(),
        "Unconfigured auth provider should be rejected: {:?}",
        result
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("nonexistent_provider") || err.contains("nonexistent"),
        "Error should mention the provider: {err}"
    );
}

#[test]
fn test_validation_crud_where_clause_sql_injection() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"

tables:
  - name: "posts"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true

endpoints:
  - path: "/api/posts"
    methods: ["get"]
    action: "crud"
    crud:
      table: "posts"
      database: "main"
      where_clause: "1 = 1; DROP TABLE posts"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(
        result.is_err(),
        "SQL injection attempt should be rejected: {:?}",
        result
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("unsafe SQL") || err.contains(";"),
        "Error should mention unsafe SQL: {err}"
    );
}

#[test]
fn test_validation_table_no_columns_rejected() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"

tables:
  - name: "empty_table"
    database: "main"
    columns: []
"#;
    let result = load_yaml(yaml);
    assert!(
        result.is_err(),
        "Table with no columns should be rejected: {:?}",
        result
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("at least one column"),
        "Error should mention columns: {err}"
    );
}

#[test]
fn test_validate_catches_schema_compilation_errors() {
    use main_serve::config::schema_registry::SchemaRegistry;

    // Config with an invalid global schema (bad JSON Schema) and a column
    // referencing it via validation_schema_ref. The config-level validation
    // passes (schema refs are structurally present), but SchemaRegistry::new
    // must catch the compile-time error.
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"

global_schemas:
  bad_schema:
    type: "this_is_not_a_valid_schema_type_xyz"

tables:
  - name: "posts"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
        nullable: false
      - name: "metadata"
        type: "jsonb"
        validation_schema_ref: "bad_schema"
"#;
    let config = load_yaml(yaml).unwrap();
    let result = SchemaRegistry::new(&config, std::path::Path::new("test_validate_schema.yaml"));
    assert!(
        result.is_err(),
        "SchemaRegistry::new must reject invalid global schema during validate"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Invalid global schema") || err.contains("bad_schema"),
        "Error should mention the invalid schema, got: {err}"
    );
}

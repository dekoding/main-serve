/// Integration tests for the file store (database-backed file catalog) endpoint.
///
/// Tests config parsing, store references, field permissions, ownership,
/// route registration, and method dispatch for the `file_store` endpoint type.
mod support;

use axum::body::Body;
use axum::http::Request;
use main_serve::config::types::EndpointAction;
use support::helpers::load_yaml;
use tower::ServiceExt;

fn root_str(dir: &tempfile::TempDir) -> String {
    dir.path().display().to_string()
}

// =============================================================================
// File store config parsing
// =============================================================================

#[test]
fn test_file_store_config_parses_basic() {
    let yaml = r#"
server:
  port: 0

stores:
  file_storage:
    backend: native
    root: "./files"

databases:
  main:
    driver: "sqlite"
    url: "file:files.db"

tables:
  - name: "file_registry"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true
      - name: "filename"
        type: "varchar"
        nullable: false

endpoints:
  - path: "/files/*"
    methods: ["get", "post", "delete"]
    action: "file_store"
    file_store:
      storage: "file_storage"
      table: "file_registry"
      database: "main"
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    assert_eq!(config.stores.len(), 1);
    let store = config.stores.get("file_storage").unwrap();
    assert_eq!(store.root.as_deref(), Some("./files"));

    let endpoint = config.endpoints.first().unwrap();
    assert!(matches!(endpoint.action, EndpointAction::FileStore));
    let fs_config = endpoint.file_store.as_ref().unwrap();
    assert_eq!(fs_config.storage, "file_storage");
    assert_eq!(fs_config.table, "file_registry");
    assert_eq!(fs_config.database, "main");
}

#[test]
fn test_file_store_config_parses_with_field_permissions() {
    let yaml = r#"
server:
  port: 0

stores:
  file_storage:
    backend: native
    root: "./files"

databases:
  main:
    driver: "sqlite"
    url: "file:files.db"

tables:
  - name: "file_registry"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/files/*"
    methods: ["get"]
    action: "file_store"
    file_store:
      storage: "file_storage"
      table: "file_registry"
      database: "main"
      field_permissions:
        filename:
          read:
            - "*"
          write:
            - admin
            - writer
        mime_type:
          read:
            - "*"
          write:
            - admin
        metadata:
          read:
            - "*"
            - reader
          write:
            - admin
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let endpoint = config.endpoints.first().unwrap();
    let fs_config = endpoint.file_store.as_ref().unwrap();
    let permissions = fs_config.field_permissions.as_ref().unwrap();
    assert_eq!(permissions.len(), 3);
    let filename_perm = permissions.get("filename").unwrap();
    assert_eq!(filename_perm.read, vec!["*".to_string()]);
    assert_eq!(
        filename_perm.write,
        vec!["admin".to_string(), "writer".to_string()]
    );
}

#[test]
fn test_file_store_config_parses_with_ownership() {
    let yaml = r#"
server:
  port: 0

stores:
  file_storage:
    backend: native
    root: "./files"

databases:
  main:
    driver: "sqlite"
    url: "file:files.db"

tables:
  - name: "file_registry"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true
      - name: "owner_id"
        type: "uuid"
        nullable: false

endpoints:
  - path: "/files/*"
    methods: ["get"]
    action: "file_store"
    file_store:
      storage: "file_storage"
      table: "file_registry"
      database: "main"
      ownership:
        owner_column: "owner_id"
        admin_override: true
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let endpoint = config.endpoints.first().unwrap();
    let fs_config = endpoint.file_store.as_ref().unwrap();
    let ownership = fs_config.ownership.as_ref().unwrap();
    assert_eq!(ownership.owner_column, "owner_id");
    assert!(ownership.admin_override);
}

#[test]
fn test_file_store_config_parses_with_trash() {
    let yaml = r#"
server:
  port: 0

stores:
  file_storage:
    backend: native
    root: "./files"

databases:
  main:
    driver: "sqlite"
    url: "file:files.db"

tables:
  - name: "file_registry"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/files/*"
    methods: ["get", "delete"]
    action: "file_store"
    file_store:
      storage: "file_storage"
      table: "file_registry"
      database: "main"
      trash:
        enabled: true
        retention_days: 90
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let endpoint = config.endpoints.first().unwrap();
    let fs_config = endpoint.file_store.as_ref().unwrap();
    let trash = fs_config.trash.as_ref().unwrap();
    assert!(trash.enabled);
    assert_eq!(trash.retention_days, 90);
}

#[test]
fn test_file_store_config_parses_with_pagination() {
    let yaml = r#"
server:
  port: 0

stores:
  file_storage:
    backend: native
    root: "./files"

databases:
  main:
    driver: "sqlite"
    url: "file:files.db"

tables:
  - name: "file_registry"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/files/*"
    methods: ["get"]
    action: "file_store"
    file_store:
      storage: "file_storage"
      table: "file_registry"
      database: "main"
      pagination:
        enabled: true
        default_page_size: 50
        max_page_size: 200
      sorting:
        enabled: true
        default_field: "filename"
        default_order: "asc"
        allowed_fields:
          - "filename"
          - "created_at"
      filtering:
        enabled: true
        allowed_fields:
          - "filename"
          - "mime_type"
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let endpoint = config.endpoints.first().unwrap();
    let fs_config = endpoint.file_store.as_ref().unwrap();
    assert!(fs_config.pagination.enabled);
    assert_eq!(fs_config.pagination.default_page_size, 50);
    assert_eq!(fs_config.pagination.max_page_size, 200);
    assert!(fs_config.sorting.enabled);
    assert_eq!(fs_config.sorting.default_field, "filename");
}

#[test]
fn test_file_store_config_parses_with_metadata_columns() {
    let yaml = r#"
server:
  port: 0

stores:
  file_storage:
    backend: native
    root: "./files"

databases:
  main:
    driver: "sqlite"
    url: "file:files.db"

tables:
  - name: "file_registry"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/files/*"
    methods: ["get"]
    action: "file_store"
    file_store:
      storage: "file_storage"
      table: "file_registry"
      database: "main"
      metadata_columns:
        - name: "mime_type"
          type: "varchar"
          nullable: false
        - name: "description"
          type: "text"
          nullable: true
        - name: "file_size"
          type: "bigint"
          nullable: false
          default: "0"
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let endpoint = config.endpoints.first().unwrap();
    let fs_config = endpoint.file_store.as_ref().unwrap();
    assert_eq!(fs_config.metadata_columns.len(), 3);
}

#[test]
fn test_file_store_config_parses_default_values() {
    let yaml = r#"
server:
  port: 0

stores:
  file_storage:
    backend: native
    root: "./files"

databases:
  main:
    driver: "sqlite"
    url: "file:files.db"

tables:
  - name: "file_registry"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/files/*"
    methods: ["get"]
    action: "file_store"
    file_store:
      storage: "file_storage"
      table: "file_registry"
      database: "main"
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let endpoint = config.endpoints.first().unwrap();
    let fs_config = endpoint.file_store.as_ref().unwrap();
    assert!(fs_config.field_permissions.is_none());
    assert!(fs_config.ownership.is_none());
    assert!(fs_config.trash.is_none());
}

// =============================================================================
// File store config validation errors
// =============================================================================

#[test]
fn test_file_store_missing_storage_field() {
    let yaml = r#"
server:
  port: 0

stores:
  file_storage:
    backend: native
    root: "./files"

databases:
  main:
    driver: "sqlite"
    url: "file:files.db"

tables:
  - name: "file_registry"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/files/*"
    methods: ["get"]
    action: "file_store"
    file_store: {}
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
fn test_file_store_missing_table_field() {
    let yaml = r#"
server:
  port: 0

stores:
  file_storage:
    backend: native
    root: "./files"

databases:
  main:
    driver: "sqlite"
    url: "file:files.db"

tables:
  - name: "file_registry"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/files/*"
    methods: ["get"]
    action: "file_store"
    file_store:
      storage: "file_storage"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("table") || err.contains("must not be empty"),
        "{err}"
    );
}

#[test]
fn test_file_store_missing_database_field() {
    let yaml = r#"
server:
  port: 0

stores:
  file_storage:
    backend: native
    root: "./files"

databases:
  main:
    driver: "sqlite"
    url: "file:files.db"

tables:
  - name: "file_registry"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/files/*"
    methods: ["get"]
    action: "file_store"
    file_store:
      storage: "file_storage"
      table: "file_registry"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("database") || err.contains("must not be empty"),
        "{err}"
    );
}

#[test]
fn test_file_store_invalid_store_ref() {
    let yaml = r#"
server:
  port: 0

stores:
  file_storage:
    backend: native
    root: "./files"

databases:
  main:
    driver: "sqlite"
    url: "file:files.db"

tables:
  - name: "file_registry"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/files/*"
    methods: ["get"]
    action: "file_store"
    file_store:
      storage: "nonexistent"
      table: "file_registry"
      database: "main"
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
fn test_file_store_invalid_table_ref() {
    let yaml = r#"
server:
  port: 0

stores:
  file_storage:
    backend: native
    root: "./files"

databases:
  main:
    driver: "sqlite"
    url: "file:files.db"

tables:
  - name: "file_registry"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/files/*"
    methods: ["get"]
    action: "file_store"
    file_store:
      storage: "file_storage"
      table: "nonexistent_table"
      database: "main"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("nonexistent_table"),
        "Error should mention missing table: {err}"
    );
}

#[test]
fn test_file_store_invalid_database_ref() {
    let yaml = r#"
server:
  port: 0

stores:
  file_storage:
    backend: native
    root: "./files"

databases:
  main:
    driver: "sqlite"
    url: "file:files.db"

tables:
  - name: "file_registry"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/files/*"
    methods: ["get"]
    action: "file_store"
    file_store:
      storage: "file_storage"
      table: "file_registry"
      database: "nonexistent_db"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("nonexistent_db"),
        "Error should mention missing database: {err}"
    );
}

// =============================================================================
// File store handler routing tests (integration)
// =============================================================================

#[tokio::test]
async fn test_file_store_unsupported_method_rejected() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let yaml = format!(
        r#"
server:
  port: 0

stores:
  file_storage:
    backend: native
    root: "{root}"

databases:
  main:
    driver: "sqlite"
    url: "file:files.db"

tables:
  - name: "file_registry"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/files/*"
    methods: ["get"]
    action: "file_store"
    file_store:
      storage: "file_storage"
      table: "file_registry"
      database: "main"
    auth: "none"
"#,
        root = root_str(&dir),
    );
    let (app, _f) = support::setup_server(&yaml).await;

    let req = Request::builder()
        .method("PUT")
        .uri("/files/abc123")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        axum::http::StatusCode::METHOD_NOT_ALLOWED
    );
}

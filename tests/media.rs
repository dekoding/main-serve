/// Integration tests for the media library endpoint handler.
///
/// Tests config parsing, store references, path dispatch, method restrictions,
/// and route registration for the media library endpoint type.
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
// Media config parsing
// =============================================================================

#[test]
fn test_media_config_parses_basic() {
    let yaml = r#"
server:
  port: 0

stores:
  media_storage:
    backend: native
    root: "./media"

databases:
  main:
    driver: "sqlite"
    url: "file:media.db"

tables:
  - name: "media_items"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true
      - name: "filename"
        type: "varchar"
        nullable: false

endpoints:
  - path: "/media/*"
    methods: ["get", "post", "delete"]
    action: "media"
    media:
      storage: "media_storage"
      table: "media_items"
      database: "main"
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    assert_eq!(config.stores.len(), 1);
    assert_eq!(config.endpoints.len(), 1);

    let endpoint = config.endpoints.first().unwrap();
    assert!(matches!(endpoint.action, EndpointAction::Media));
    let media_config = endpoint.media.as_ref().unwrap();
    assert_eq!(media_config.storage, "media_storage");
    assert_eq!(media_config.table, "media_items");
    assert_eq!(media_config.database, "main");
}

#[test]
fn test_media_config_parses_with_all_options() {
    let yaml = r#"
server:
  port: 0

stores:
  media_storage:
    backend: native
    root: "./media"

databases:
  main:
    driver: "sqlite"
    url: "file:media.db"

tables:
  - name: "media_items"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/media/*"
    methods: ["get", "post", "patch", "delete"]
    action: "media"
    media:
      storage: "media_storage"
      table: "media_items"
      database: "main"
      columns:
        - "auto"
        - "tags"
        - "description"
        - "alt_text"
        - "content_type"
      upload:
        max_size: 10485760
        allowed_extensions:
          - ".jpg"
          - ".png"
          - ".gif"
          - ".webp"
      trash:
        enabled: true
        retention_days: 30
      sharing:
        enabled: true
        prefix: "shared"
        default_ttl: 604800
      image_resize:
         enabled: true
         cache_dir: "_resized"
         max_dimension: 4096
      pagination:
        enabled: true
        default_page_size: 20
        max_page_size: 100
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let endpoint = config.endpoints.first().unwrap();
    let media_config = endpoint.media.as_ref().unwrap();
    assert_eq!(media_config.columns.len(), 5);
    assert!(media_config.upload.is_some());
    assert!(media_config.trash.is_some());
    assert!(media_config.sharing.is_some());
    assert!(media_config.image_resize.is_some());
    let upload = media_config.upload.as_ref().unwrap();
    assert_eq!(upload.max_size, 10_485_760);
    let trash = media_config.trash.as_ref().unwrap();
    assert!(trash.enabled);
    assert_eq!(trash.retention_days, 30);
}

#[test]
fn test_media_config_parses_with_metadata_columns() {
    let yaml = r#"
server:
  port: 0

stores:
  media_storage:
    backend: native
    root: "./media"

databases:
  main:
    driver: "sqlite"
    url: "file:media.db"

tables:
  - name: "media_items"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/media/*"
    methods: ["get"]
    action: "media"
    media:
      storage: "media_storage"
      table: "media_items"
      database: "main"
      metadata_columns:
        - name: "title"
          type: "varchar"
          nullable: false
        - name: "description"
          type: "text"
          nullable: true
        - name: "views"
          type: "integer"
          nullable: false
          default: "0"
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let endpoint = config.endpoints.first().unwrap();
    let media_config = endpoint.media.as_ref().unwrap();
    assert_eq!(media_config.metadata_columns.len(), 3);
    let title_col = media_config.metadata_columns.first().unwrap();
    assert_eq!(title_col.name, "title");
    assert_eq!(title_col.column_type, "varchar");
    assert!(!title_col.nullable);
}

#[test]
fn test_media_config_parses_with_user_scope() {
    let yaml = r#"
server:
  port: 0

stores:
  media_storage:
    backend: native
    root: "./media"

databases:
  main:
    driver: "sqlite"
    url: "file:media.db"

tables:
  - name: "media_items"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/media/*"
    methods: ["get"]
    action: "media"
    media:
      storage: "media_storage"
      table: "media_items"
      database: "main"
      user_scope:
        enabled: true
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let endpoint = config.endpoints.first().unwrap();
    let media_config = endpoint.media.as_ref().unwrap();
    let scope = media_config.user_scope.as_ref().unwrap();
    assert!(scope.enabled);
}

#[test]
fn test_media_config_parses_with_content_references() {
    let yaml = r#"
server:
  port: 0

stores:
  media_storage:
    backend: native
    root: "./media"

databases:
  main:
    driver: "sqlite"
    url: "file:media.db"

tables:
  - name: "media_items"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/media/*"
    methods: ["get"]
    action: "media"
    media:
      storage: "media_storage"
      table: "media_items"
      database: "main"
      content_references:
        table: "media_content_references"
        on_delete: "detach"
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let endpoint = config.endpoints.first().unwrap();
    let media_config = endpoint.media.as_ref().unwrap();
    let refs = media_config.content_references.as_ref().unwrap();
    assert_eq!(refs.table, "media_content_references");
    assert!(matches!(
        refs.on_delete,
        main_serve::config::types::MediaOnDeleteBehavior::Detach
    ));
}

// =============================================================================
// Media config validation errors
// =============================================================================

#[test]
fn test_media_missing_storage_field() {
    let yaml = r#"
server:
  port: 0

stores:
  media_storage:
    backend: native
    root: "./media"

databases:
  main:
    driver: "sqlite"
    url: "file:media.db"

tables:
  - name: "media_items"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/media/*"
    methods: ["get"]
    action: "media"
    media: {}
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
fn test_media_missing_table_field() {
    let yaml = r#"
server:
  port: 0

stores:
  media_storage:
    backend: native
    root: "./media"

databases:
  main:
    driver: "sqlite"
    url: "file:media.db"

tables:
  - name: "media_items"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/media/*"
    methods: ["get"]
    action: "media"
    media:
      storage: "media_storage"
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
fn test_media_missing_database_field() {
    let yaml = r#"
server:
  port: 0

stores:
  media_storage:
    backend: native
    root: "./media"

databases:
  main:
    driver: "sqlite"
    url: "file:media.db"

tables:
  - name: "media_items"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/media/*"
    methods: ["get"]
    action: "media"
    media:
      storage: "media_storage"
      table: "media_items"
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
fn test_media_invalid_store_ref() {
    let yaml = r#"
server:
  port: 0

stores:
  media_storage:
    backend: native
    root: "./media"

databases:
  main:
    driver: "sqlite"
    url: "file:media.db"

tables:
  - name: "media_items"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/media/*"
    methods: ["get"]
    action: "media"
    media:
      storage: "nonexistent"
      table: "media_items"
      database: "main"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("nonexistent"), "{err}");
}

#[test]
fn test_media_invalid_table_ref() {
    let yaml = r#"
server:
  port: 0

stores:
  media_storage:
    backend: native
    root: "./media"

databases:
  main:
    driver: "sqlite"
    url: "file:media.db"

tables:
  - name: "media_items"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/media/*"
    methods: ["get"]
    action: "media"
    media:
      storage: "media_storage"
      table: "nonexistent_table"
      database: "main"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("nonexistent_table"), "{err}");
}

#[test]
fn test_media_invalid_database_ref() {
    let yaml = r#"
server:
  port: 0

stores:
  media_storage:
    backend: native
    root: "./media"

databases:
  main:
    driver: "sqlite"
    url: "file:media.db"

tables:
  - name: "media_items"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/media/*"
    methods: ["get"]
    action: "media"
    media:
      storage: "media_storage"
      table: "media_items"
      database: "nonexistent_db"
    auth: "none"
"#;
    let result = load_yaml(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("nonexistent_db"), "{err}");
}

#[test]
fn test_media_config_parses_facets() {
    let yaml = r#"
server:
  port: 0

stores:
  media_storage:
    backend: native
    root: "./media"

databases:
  main:
    driver: "sqlite"
    url: "file:media.db"

tables:
  - name: "media_items"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/media/*"
    methods: ["get"]
    action: "media"
    media:
      storage: "media_storage"
      table: "media_items"
      database: "main"
      facets:
        - name: "Content Type"
          field: "content_type"
          facet_type: term
        - name: "MIME Type"
          field: "mime_type"
          facet_type: term
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let endpoint = config.endpoints.first().unwrap();
    let media_config = endpoint.media.as_ref().unwrap();
    assert_eq!(media_config.facets.len(), 2);
    assert_eq!(media_config.facets[0].field, "content_type");
}

// =============================================================================
// Media handler routing tests (integration)
// =============================================================================

#[tokio::test]
async fn test_media_unsupported_method_rejected() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let yaml = format!(
        r#"
server:
  port: 0

stores:
  media_storage:
    backend: native
    root: "{root}"

databases:
  main:
    driver: "sqlite"
    url: "file:media.db"

tables:
  - name: "media_items"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true

endpoints:
  - path: "/media/*"
    methods: ["get"]
    action: "media"
    media:
      storage: "media_storage"
      table: "media_items"
      database: "main"
    auth: "none"
"#,
        root = root_str(&dir),
    );
    let (app, _f) = support::setup_server(&yaml).await;

    // PUT is not in methods list
    let req = Request::builder()
        .method("PUT")
        .uri("/media/abc123")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        axum::http::StatusCode::METHOD_NOT_ALLOWED
    );
}

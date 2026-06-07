// =============================================================================
// CRUD operations configs
// =============================================================================

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
  - name: "__TABLE_NAME__"
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
      pagination:
        enabled: true
        default_page_size: 2
        max_page_size: 10
      filtering:
        enabled: true
        allowed_fields: ["author"]
      sorting:
        enabled: true
        allowed_fields: ["title", "id"]
        default_field: "id"
        default_order: "asc"
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

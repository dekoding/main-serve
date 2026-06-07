// =============================================================================
// Shared configs
// =============================================================================

pub const MINIMAL_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

endpoints:
  - path: "/ping"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "text/plain"
      body: "pong"
    auth: "none"
"#;

/// Minimal config with a single public `custom_response` endpoint.
///
/// Equivalent to `MINIMAL_CONFIG` but with JSON body and no `content_type` override.
/// Used by tests that only care about a basic working endpoint.
pub const MINIMAL_JSON_CONFIG: &str = r#"
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

// =============================================================================
// JSONB configs - shared across crud_operations.rs and sql_expression_injection.rs
// =============================================================================

/// JSONB-enabled posts table config for expression testing.
///
/// Contains `__DB_DRIVER__`, `__DB_URL__`, and `__TABLE_NAME__` placeholders
/// that `TestDatabase::setup_app()` replaces automatically.
pub const JSONB_EXPRESSIONS_CONFIG: &str = r#"
server:
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
      - name: "metadata"
        type: "jsonb"
        nullable: true
      - name: "author"
        type: "varchar"
        nullable: true

endpoints:
  - path: "/api/posts"
    methods: ["get", "post"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "metadata", "author"]
      writable_fields: ["title", "metadata", "author"]
      filtering:
        enabled: true
        allowed_fields: ["title", "author", "metadata.role"]
      sorting:
        enabled: true
        allowed_fields: ["title", "author", "metadata", "metadata.role"]
        default_field: "id"
        default_order: "asc"
    auth: "none"
"#;

/// Comprehensive JSONB config supporting multiple operators, nested paths,
/// bracket notation, and multi-column JSONB filtering/sorting across all
/// supported backends.
pub const JSONB_FILTER_SORT_CONFIG: &str = r#"
server:
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
      - name: "metadata"
        type: "jsonb"
        nullable: true
      - name: "tags"
        type: "jsonb"
        nullable: true
      - name: "author"
        type: "varchar"
        nullable: true

endpoints:
  - path: "/api/posts"
    methods: ["get", "post"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "metadata", "tags", "author"]
      writable_fields: ["title", "metadata", "tags", "author"]
      pagination:
        enabled: true
        default_page_size: 20
        max_page_size: 100
      filtering:
        enabled: true
        allowed_fields: [
          "metadata.role",
          "metadata.user.age",
          "metadata.status",
          "metadata",
          "tags",
          "title",
          "author"
        ]
      sorting:
        enabled: true
        allowed_fields: ["title", "author", "metadata.role", "metadata.user.age", "metadata", "id"]
        default_field: "id"
        default_order: "asc"
    auth: "none"

  - path: "/api/posts/{id}"
    methods: ["get", "put", "delete"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "metadata", "tags", "author"]
      writable_fields: ["title", "metadata", "tags", "author"]
    auth: "none"
"#;

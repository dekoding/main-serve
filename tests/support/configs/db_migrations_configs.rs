// =============================================================================
// DB migration configs
// =============================================================================

/// V1: table with `id` + `title` columns. No endpoints.
pub const MIGRATION_ADD_COLUMN_V1: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: false

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

endpoints: []
"#;

/// V2: table with `id` + `title` + `status` columns. No endpoints.
pub const MIGRATION_ADD_COLUMN_V2: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: false

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
      - name: "status"
        type: "varchar"
        nullable: false
        default: "'draft'"

endpoints: []
"#;

/// Base: table with `id` + `title` + `body` columns.
///
/// Contains `__ALLOW_DESTRUCTIVE__` placeholder for tests that need
/// to set the value dynamically.
pub const MIGRATION_DROP_COLUMN_BASE: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: __ALLOW_DESTRUCTIVE__

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

endpoints: []
"#;

/// Target: table with `id` + `title` only, `allow_destructive`: true.
pub const MIGRATION_DROP_COLUMN_TARGET_DESTRUCTIVE: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: __ALLOW_DESTRUCTIVE__

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

endpoints: []
"#;

pub const INDEX_CONFIG: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: false

tables:
  - name: "__TABLE_NAME__"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "email"
        type: "varchar"
        nullable: false
        indexed: true

endpoints: []
"#;

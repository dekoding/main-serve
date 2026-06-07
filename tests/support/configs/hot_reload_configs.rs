// =============================================================================
// Hot reload configs
// =============================================================================

pub const RELOAD_DB_V1: &str = r#"
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

pub const RELOAD_DB_V2_ADD: &str = r#"
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

pub const RELOAD_DB_V1_DROP: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: true

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

pub const RELOAD_DB_V2_DROP: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: true

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

pub const HOT_RELOAD_MINIMAL_CONFIG: &str = r#"
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

/// Config for testing that reload adds a second endpoint.
/// Contains two `custom_response` endpoints.
pub const HOT_RELOAD_RELOAD_TARGET: &str = r#"
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
"#;

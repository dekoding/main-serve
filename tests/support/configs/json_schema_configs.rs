// =============================================================================
// JSON schema validation configs
// =============================================================================

/// Config template with an inline JSON Schema validation on the `metadata` JSONB column.
///
/// The schema requires `metadata` to be an object with a `role` string field
/// when present. Used by tests that exercise inline schema validation.
pub const JSONB_INLINE_SCHEMA_CONFIG: &str = r#"
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
        validation:
          type: object
          properties:
            role:
              type: string
            status:
              type: string
          additionalProperties: false

endpoints:
  - path: "/api/posts"
    methods: ["get", "post"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "metadata"]
      writable_fields: ["title", "metadata"]
    auth: "none"

  - path: "/api/posts/{id}"
    methods: ["get", "put", "delete"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "metadata"]
      writable_fields: ["title", "metadata"]
    auth: "none"
"#;

/// Config template with a global schema referenced by the `metadata` JSONB column.
///
/// The `user_profile` global schema requires `name` (string) and `email` (string).
pub const JSONB_GLOBAL_REF_SCHEMA_CONFIG: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true

global_schemas:
  user_profile:
    type: object
    required:
      - name
      - email
    properties:
      name:
        type: string
      email:
        type: string

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
      - name: "user_info"
        type: "jsonb"
        nullable: true
        validation_schema_ref: "user_profile"

endpoints:
  - path: "/api/posts"
    methods: ["get", "post"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "user_info"]
      writable_fields: ["title", "user_info"]
    auth: "none"

  - path: "/api/posts/{id}"
    methods: ["get", "put", "delete"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "user_info"]
      writable_fields: ["title", "user_info"]
    auth: "none"
"#;

/// Config template with a JSON Schema validation on an external file for the
/// `metadata` JSONB column. The file path is resolved relative to the config file.
///
/// The schema file is expected at `{config_dir}/jsonb_validation_schema.json`.
/// Use with a test that writes this file to the same directory as the config.
pub const JSONB_EXTERNAL_SCHEMA_CONFIG: &str = r#"
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
        validation_schema: "jsonb_validation_schema.json"

endpoints:
  - path: "/api/posts"
    methods: ["get", "post"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "metadata"]
      writable_fields: ["title", "metadata"]
    auth: "none"

  - path: "/api/posts/{id}"
    methods: ["get", "put", "delete"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "metadata"]
      writable_fields: ["title", "metadata"]
    auth: "none"
"#;

/// Config template without any JSON Schema validation.
///
/// Used as a control config to verify that requests with arbitrary JSONB data
/// are accepted when no schema is configured.
pub const JSONB_NO_SCHEMA_CONFIG: &str = r#"
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

endpoints:
  - path: "/api/posts"
    methods: ["get", "post"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "metadata"]
      writable_fields: ["title", "metadata"]
    auth: "none"

  - path: "/api/posts/{id}"
    methods: ["get", "put", "delete"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "metadata"]
      writable_fields: ["title", "metadata"]
    auth: "none"
"#;

/// Config template with multiple JSONB columns, each having different schema
/// validation mechanisms (inline, global ref, and no schema).
///
/// Used to test that validation runs for all schema-validated JSONB columns
/// and that non-schema columns are unaffected.
pub const JSONB_MULTIPLE_COLUMNS_CONFIG: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true

global_schemas:
  tags_schema:
    type: array
    items:
      type: string

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
        validation:
          type: object
          properties:
            role:
              type: string
          required:
            - role
          additionalProperties: false
      - name: "tags"
        type: "jsonb"
        nullable: true
        validation_schema_ref: "tags_schema"

endpoints:
  - path: "/api/posts"
    methods: ["get", "post"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "metadata", "tags"]
      writable_fields: ["title", "metadata", "tags"]
    auth: "none"

  - path: "/api/posts/{id}"
    methods: ["get", "put", "delete"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
      fields: ["id", "title", "metadata", "tags"]
      writable_fields: ["title", "metadata", "tags"]
    auth: "none"
"#;

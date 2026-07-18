/// JSON schema validation config loading tests.
///
/// Tests that schemas defined via `global_schemas`, inline `validation`,
/// external `validation_schema` files, and `validation_schema_ref` references
/// are correctly parsed by `AppConfig` and built into the `SchemaRegistry`.
mod support;
use std::io::Write;
use tempfile::NamedTempFile;

use main_serve::config::schema_registry::SchemaRegistry;
use main_serve::config::types::{ColumnConfig, ColumnType};

/// Helper to write YAML to a temp file and load it through the full config
/// pipeline (including SchemaRegistry construction).
fn load_yaml_with_registry(
    yaml: &str,
) -> Result<main_serve::config::AppConfig, main_serve::error::AppError> {
    let mut f = NamedTempFile::new().expect("failed to create temp file");
    f.write_all(yaml.as_bytes()).expect("failed to write");
    let config = main_serve::config::load_config(f.path())?;
    // Build the schema registry as part of the loading pipeline
    let _registry = SchemaRegistry::new(&config, f.path())?;
    Ok(config)
}

// =========================================================================
// Column config serialization tests (Unit-style)
// =========================================================================

#[test]
fn test_column_config_has_validation_fields() {
    let col = ColumnConfig {
        name: "metadata".to_string(),
        column_type: ColumnType::Jsonb,
        primary_key: false,
        nullable: true,
        default: None,
        unique: false,
        indexed: false,
        validation_schema: Some("./schemas/meta.json".to_string()),
        validation: None,
        validation_schema_ref: None,
    };
    assert_eq!(
        col.validation_schema.as_deref(),
        Some("./schemas/meta.json")
    );
}

#[test]
fn test_column_config_inline_validation() {
    let col = ColumnConfig {
        name: "metadata".to_string(),
        column_type: ColumnType::Jsonb,
        primary_key: false,
        nullable: true,
        default: None,
        unique: false,
        indexed: false,
        validation_schema: None,
        validation: Some(
            serde_yaml::from_value(serde_yaml::Value::Mapping(serde_yaml::Mapping::new())).unwrap(),
        ),
        validation_schema_ref: None,
    };
    assert!(col.validation.is_some());
}

#[test]
fn test_column_config_global_ref() {
    let col = ColumnConfig {
        name: "content".to_string(),
        column_type: ColumnType::Jsonb,
        primary_key: false,
        nullable: true,
        default: None,
        unique: false,
        indexed: false,
        validation_schema: None,
        validation: None,
        validation_schema_ref: Some("global_schemas.blog_post".to_string()),
    };
    assert_eq!(
        col.validation_schema_ref.as_deref(),
        Some("global_schemas.blog_post")
    );
}

// =========================================================================
// Global schemas section tests
// =========================================================================

#[test]
fn test_load_global_schemas() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"

global_schemas:
  blog_post:
    type: "object"
    required:
      - title
      - body
    properties:
      title: { type: "string" }
      body: { type: "string" }
  user_profile:
    type: "object"
    properties:
      name: { type: "string" }
      email: { type: "string" }
"#;
    let result = load_yaml_with_registry(yaml);
    let config = result.expect("config should load");
    let schemas = config
        .global_schemas
        .as_ref()
        .expect("global_schemas should be present");
    assert_eq!(schemas.len(), 2);
    assert!(schemas.contains_key("blog_post"));
    assert!(schemas.contains_key("user_profile"));
}

#[test]
fn test_load_global_schemas_invalid_schema_rejected() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"

global_schemas:
  bad_schema:
    type: "invalid_type_that_doesnt_exist_12345"
"#;
    let result = load_yaml_with_registry(yaml);
    assert!(
        result.is_err(),
        "invalid global schema should be rejected at load time"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Invalid global schema") || err.contains("bad_schema"),
        "Error should mention invalid schema, got: {err}"
    );
}

#[test]
fn test_columns_without_validation_unchanged() {
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
      - name: "bio"
        type: "text"
"#;
    let config = load_yaml_with_registry(yaml).unwrap();
    let users = config.tables.iter().find(|t| t.name == "users").unwrap();
    assert_eq!(users.columns.len(), 3);
    for col in &users.columns {
        assert!(col.validation_schema.is_none());
        assert!(col.validation.is_none());
        assert!(col.validation_schema_ref.is_none());
    }
}

// =========================================================================
// Inline validation schema tests
// =========================================================================

#[test]
fn test_load_columns_with_inline_validation() {
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
        nullable: false
      - name: "content"
        type: "jsonb"
        validation:
          type: object
          required:
            - title
            - body
          properties:
            title: { type: string }
            body: { type: string }
"#;
    let config = load_yaml_with_registry(yaml).unwrap();
    let posts = config.tables.iter().find(|t| t.name == "posts").unwrap();
    let content_col = posts.columns.iter().find(|c| c.name == "content").unwrap();
    assert!(
        content_col.validation.is_some(),
        "should have inline validation schema"
    );
}

// =========================================================================
// External schema file reference tests
// =========================================================================

#[test]
fn test_load_columns_with_validation_schema_file_not_found() {
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
        nullable: false
      - name: "content"
        type: "jsonb"
        validation_schema: "./nonexistent_schema.json"
"#;
    let result = load_yaml_with_registry(yaml);
    assert!(
        result.is_err(),
        "external schema file that doesn't exist should fail"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("nonexistent")
            || err.contains("not found")
            || err.contains("Failed to resolve"),
        "Error should mention the missing file, got: {err}"
    );
}

#[test]
fn test_load_columns_with_missing_global_schema_ref() {
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
        nullable: false
      - name: "content"
        type: "jsonb"
        validation_schema_ref: "global_schemas.nonexistent_schema"
"#;
    let result = load_yaml_with_registry(yaml);
    assert!(
        result.is_err(),
        "missing global schema reference should fail"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("nonexistent") || err.contains("not found"),
        "Error should mention the missing schema, got: {err}"
    );
}

// =========================================================================
// Full config end-to-end tests
// =========================================================================

#[test]
fn test_load_config_with_global_schemas_and_column_refs() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"

global_schemas:
  blog_post:
    type: "object"
    required: [title, body]
    properties:
      title: { type: string }
      body: { type: string }

tables:
  - name: "posts"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
        nullable: false
      - name: "content"
        type: "jsonb"
        validation_schema_ref: "global_schemas.blog_post"
"#;
    let config = load_yaml_with_registry(yaml).unwrap();
    let posts = config.tables.iter().find(|t| t.name == "posts").unwrap();
    let content_col = posts.columns.iter().find(|c| c.name == "content").unwrap();
    assert!(
        content_col.validation_schema_ref.is_some(),
        "column should have validation_schema_ref"
    );
}

#[test]
fn test_load_config_with_external_schema_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let schema_path = dir.path().join("post_schema.json");
    std::fs::write(
        &schema_path,
        r#"{"type": "object", "required": ["title"], "properties": {"title": {"type": "string" }}}
"#,
    )
    .unwrap();

    let config_yaml = format!(
        r#"
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
        nullable: false
      - name: "content"
        type: "jsonb"
        validation_schema: "{}"
"#,
        schema_path.file_name().unwrap().to_str().unwrap()
    );

    // Write config to the same temp dir so relative path resolution works
    let config_path = dir.path().join("config.yaml");
    std::fs::write(&config_path, &config_yaml).unwrap();

    let config = main_serve::config::load_config(&config_path).unwrap();
    // Build the schema registry to validate the external schema resolves
    let _registry =
        SchemaRegistry::new(&config, &config_path).expect("should build schema registry");
    let posts = config.tables.iter().find(|t| t.name == "posts").unwrap();
    let content_col = posts.columns.iter().find(|c| c.name == "content").unwrap();
    assert!(
        content_col.validation_schema.is_some(),
        "column should have validation_schema"
    );
}

#[test]
fn test_load_config_with_reference_to_missing_global_schema() {
    let yaml = r#"
databases:
  main:
    driver: "sqlite"
    url: "sqlite://test.db"

global_schemas:
  existing_schema:
    type: "object"
    properties:
      name: { type: string }

tables:
  - name: "posts"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
        nullable: false
      - name: "content"
        type: "jsonb"
        validation_schema_ref: "global_schemas.missing_schema"
"#;
    let result = load_yaml_with_registry(yaml);
    assert!(
        result.is_err(),
        "reference to missing global schema should fail"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("missing_schema"),
        "Error should mention the missing schema name, got: {err}"
    );
}

#[test]
fn test_load_config_with_external_schema_that_is_invalid() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let schema_path = dir.path().join("invalid_schema.json");
    std::fs::write(
        &schema_path,
        r#"{"type": "invalid_non_existent_type_xyz"}
"#,
    )
    .unwrap();

    let config_yaml = format!(
        r#"
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
        nullable: false
      - name: "content"
        type: "jsonb"
        validation_schema: "{}"
"#,
        schema_path.file_name().unwrap().to_str().unwrap()
    );

    let config_path = dir.path().join("config.yaml");
    std::fs::write(&config_path, &config_yaml).unwrap();

    let result = main_serve::config::load_config(&config_path);
    assert!(result.is_ok(), "config load itself should succeed");

    let config = result.unwrap();
    let registry_result = SchemaRegistry::new(&config, &config_path);
    assert!(
        registry_result.is_err(),
        "invalid external schema should fail at registry build"
    );
    let err = registry_result.unwrap_err().to_string();
    assert!(
        err.contains("Invalid schema") || err.contains("invalid"),
        "Error should mention invalid schema, got: {err}"
    );
}

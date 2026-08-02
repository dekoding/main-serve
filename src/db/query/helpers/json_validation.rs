//! JSON Schema validation utilities for JSONB columns.
//!
//! Validates JSON values in request bodies against per-column JSON schemas
//! configured in the YAML config. Validation runs at the handler level,
//! before SQL building, to catch schema violations with clear error messages.
use serde_json::Value;

use crate::config::schema_registry::{SchemaRegistry, validate_json_with_schema};
use crate::error::AppError;

/// Validate JSONB column values in a request body against their configured schemas.
///
/// Returns `Ok(())` if all validated columns pass, or `AppError::JsonValidationError`
/// with a summary message and per-column violation details if any column fails.
/// Non-JSONB columns and columns without a schema are silently skipped.
///
/// The `body` may contain arbitrary keys (not just JSONB columns). Only the
/// intersection of body keys and schema-validated JSONB columns is validated.
pub fn validate_jsonb_body(
    body: &Value,
    table_name: &str,
    schema_registry: &SchemaRegistry,
) -> Result<(), AppError> {
    let Some(validators) = schema_registry.get_table_schema(table_name) else {
        return Ok(());
    };

    let mut errors: Vec<String> = Vec::new();

    for (column_name, schema) in &validators {
        if let Some(value) = body.get(column_name) {
            let col_errors = validate_json_with_schema(value, schema);
            if !col_errors.is_empty() {
                for err in col_errors {
                    errors.push(format!("{column_name} ({}): {err}", schema.source()));
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        let summary = format!("JSONB validation failed: {}", errors[0]);
        let details = if errors.len() > 1 {
            errors[1..].to_vec()
        } else {
            Vec::new()
        };
        Err(AppError::JsonValidationError {
            message: summary,
            details,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;
    use crate::config::types::{ColumnType, DatabaseConfig, DatabaseDriver, TableConfig};
    use crate::error::AppError;

    fn make_registry_with_inline_schema(schema_json: &serde_yaml::Value) -> SchemaRegistry {
        let inline = serde_yaml::to_value(schema_json).unwrap();
        let config = crate::config::types::AppConfig {
            databases: {
                let mut m = HashMap::new();
                m.insert(
                    "main".to_string(),
                    DatabaseConfig {
                        driver: DatabaseDriver::Sqlite,
                        url: "sqlite://:memory:".to_string(),
                        min_connections: 1,
                        max_connections: 10,
                        auto_migrate: false,
                        allow_destructive: false,
                        acquire_timeout: 5,
                    },
                );
                m
            },
            tables: vec![TableConfig {
                name: "test_table".to_string(),
                database: "main".to_string(),
                columns: vec![],
                foreign_keys: vec![],
            }],
            global_schemas: Some({
                let mut m = HashMap::new();
                m.insert("test_schema".to_string(), inline);
                m
            }),
            ..Default::default()
        };
        SchemaRegistry::new(&config, &std::path::PathBuf::from("test.yaml")).unwrap()
    }

    fn make_registry_with_column_schema(
        column_name: &str,
        schema_json: &serde_yaml::Value,
    ) -> SchemaRegistry {
        let inline = serde_yaml::to_value(schema_json).unwrap();
        let config = crate::config::types::AppConfig {
            databases: {
                let mut m = HashMap::new();
                m.insert(
                    "main".to_string(),
                    DatabaseConfig {
                        driver: DatabaseDriver::Sqlite,
                        url: "sqlite://:memory:".to_string(),
                        min_connections: 1,
                        max_connections: 10,
                        auto_migrate: false,
                        allow_destructive: false,
                        acquire_timeout: 5,
                    },
                );
                m
            },
            tables: vec![TableConfig {
                name: "test_table".to_string(),
                database: "main".to_string(),
                columns: vec![crate::config::types::ColumnConfig {
                    name: column_name.to_string(),
                    column_type: ColumnType::Jsonb,
                    primary_key: false,
                    nullable: true,
                    default: None,
                    unique: false,
                    indexed: false,
                    validation_schema: None,
                    validation: Some(inline),
                    validation_schema_ref: None,
                }],
                foreign_keys: vec![],
            }],
            ..Default::default()
        };
        SchemaRegistry::new(&config, &std::path::PathBuf::from("test.yaml")).unwrap()
    }

    #[test]
    fn test_validate_jsonb_column_valid_data() {
        let schema: serde_yaml::Value = serde_yaml::from_str(
            r"
type: object
required:
  - name
  - role
properties:
  name:
    type: string
  role:
    type: string
",
        )
        .unwrap();

        let registry = make_registry_with_column_schema("data", &schema);
        let body = json!({"data": {"name": "Alice", "role": "admin"}});

        let result = validate_jsonb_body(&body, "test_table", &registry);
        assert!(result.is_ok(), "valid data should pass: {result:?}");
    }

    #[test]
    fn test_validate_jsonb_column_missing_required_field() {
        let schema: serde_yaml::Value = serde_yaml::from_str(
            r"
type: object
required:
  - name
  - email
properties:
  name:
    type: string
  email:
    type: string
",
        )
        .unwrap();

        let registry = make_registry_with_column_schema("metadata", &schema);
        let body = json!({"metadata": {"name": "Bob"}, "other_field": "ignored"});

        let result = validate_jsonb_body(&body, "test_table", &registry);
        assert!(result.is_err(), "missing required field should fail");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("email"),
            "Error should mention missing 'email', got: {err_msg}"
        );
    }

    #[test]
    fn test_validate_jsonb_column_wrong_type() {
        let schema: serde_yaml::Value = serde_yaml::from_str(
            r"
type: object
required:
  - title
properties:
  title:
    type: string
",
        )
        .unwrap();

        let registry = make_registry_with_column_schema("content", &schema);
        let body = json!({"content": "not an object"});

        let result = validate_jsonb_body(&body, "test_table", &registry);
        assert!(result.is_err(), "wrong type should fail");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("type"),
            "Error should mention type, got: {err_msg}"
        );
    }

    #[test]
    fn test_validate_jsonb_column_additional_properties() {
        let schema: serde_yaml::Value = serde_yaml::from_str(
            r"
type: object
required:
  - name
properties:
  name:
    type: string
additionalProperties: false
",
        )
        .unwrap();

        let registry = make_registry_with_column_schema("data", &schema);
        let body = json!({"data": {"name": "Alice", "extra_field": "should fail"}});

        let result = validate_jsonb_body(&body, "test_table", &registry);
        assert!(result.is_err(), "additional properties should fail");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("additional") || err_msg.contains("Additional properties"),
            "Error should mention additional properties, got: {err_msg}"
        );
    }

    #[test]
    fn test_validate_jsonb_column_invalid_enum() {
        let schema: serde_yaml::Value = serde_yaml::from_str(
            r"
type: object
required:
  - status
properties:
  status:
    type: string
    enum:
      - active
      - inactive
",
        )
        .unwrap();

        let registry = make_registry_with_column_schema("meta", &schema);
        let body = json!({"meta": {"status": "unknown"}});

        let result = validate_jsonb_body(&body, "test_table", &registry);
        assert!(result.is_err(), "invalid enum value should fail");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("enum") || err_msg.contains("unknown"),
            "Error should mention enum or the invalid value, got: {err_msg}"
        );
    }

    #[test]
    fn test_validate_jsonb_column_nested_errors() {
        let schema: serde_yaml::Value = serde_yaml::from_str(
            r"
type: object
required:
  - profile
properties:
  profile:
    type: object
    required:
      - name
      - age
    properties:
      name:
        type: string
      age:
        type: integer
",
        )
        .unwrap();

        let registry = make_registry_with_column_schema("user_data", &schema);
        let body = json!({"user_data": {"profile": {"name": 123}}});

        let result = validate_jsonb_body(&body, "test_table", &registry);
        assert!(result.is_err(), "nested type error should fail");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("profile") || err_msg.contains("age"),
            "Error should mention nested path, got: {err_msg}"
        );
    }

    #[test]
    fn test_validate_jsonb_column_no_schema_skips_validation() {
        let registry =
            make_registry_with_inline_schema(&serde_yaml::from_str("type: object").unwrap());
        // No JSONB columns registered in the table, only a global schema
        let body = json!({"anything": "goes"});

        let result = validate_jsonb_body(&body, "test_table", &registry);
        assert!(
            result.is_ok(),
            "no schema on table columns should skip validation"
        );
    }

    #[test]
    fn test_validate_jsonb_body_skips_non_jsonb_keys() {
        // The body contains a key that is not a JSONB column in the table.
        // Non-JSONB keys should be ignored entirely.
        let schema: serde_yaml::Value = serde_yaml::from_str(
            r"
type: object
required:
  - name
properties:
  name:
    type: string
",
        )
        .unwrap();

        let registry = make_registry_with_column_schema("settings", &schema);
        // "settings" is not in the body, but a non-JSONB column "name" is
        let body = json!({"name": "top-level field not a jsonb column"});

        let result = validate_jsonb_body(&body, "test_table", &registry);
        assert!(
            result.is_ok(),
            "non-JSONB body keys should be silently ignored"
        );
    }

    #[test]
    fn test_validate_jsonb_body_multiple_columns_multiple_errors() {
        // Create a table with two JSONB columns, both with schemas, both failing
        let schema_a: serde_yaml::Value = serde_yaml::from_str(
            r"
type: object
required:
  - x
properties:
  x:
    type: string
",
        )
        .unwrap();

        let schema_b: serde_yaml::Value = serde_yaml::from_str(
            r"
type: object
required:
  - y
properties:
  y:
    type: string
",
        )
        .unwrap();

        let inline_a = serde_yaml::to_value(&schema_a).unwrap();
        let inline_b = serde_yaml::to_value(&schema_b).unwrap();

        let config = crate::config::types::AppConfig {
            databases: {
                let mut m = HashMap::new();
                m.insert(
                    "main".to_string(),
                    DatabaseConfig {
                        driver: DatabaseDriver::Sqlite,
                        url: "sqlite://:memory:".to_string(),
                        min_connections: 1,
                        max_connections: 10,
                        auto_migrate: false,
                        allow_destructive: false,
                        acquire_timeout: 5,
                    },
                );
                m
            },
            tables: vec![TableConfig {
                name: "test_table".to_string(),
                database: "main".to_string(),
                columns: vec![
                    crate::config::types::ColumnConfig {
                        name: "col_a".to_string(),
                        column_type: ColumnType::Jsonb,
                        primary_key: false,
                        nullable: true,
                        default: None,
                        unique: false,
                        indexed: false,
                        validation_schema: None,
                        validation: Some(inline_a),
                        validation_schema_ref: None,
                    },
                    crate::config::types::ColumnConfig {
                        name: "col_b".to_string(),
                        column_type: ColumnType::Jsonb,
                        primary_key: false,
                        nullable: true,
                        default: None,
                        unique: false,
                        indexed: false,
                        validation_schema: None,
                        validation: Some(inline_b),
                        validation_schema_ref: None,
                    },
                ],
                foreign_keys: vec![],
            }],
            ..Default::default()
        };
        let registry =
            SchemaRegistry::new(&config, &std::path::PathBuf::from("test.yaml")).unwrap();

        let body = json!({"col_a": {}, "col_b": {}});
        let result = validate_jsonb_body(&body, "test_table", &registry);
        assert!(result.is_err(), "both columns should fail");
        let err = result.unwrap_err();
        let AppError::JsonValidationError { message, details } = err else {
            panic!("expected JsonValidationError");
        };
        // The first error goes into message; remaining errors go into details.
        // Since HashMap iteration order is non-deterministic, check that col_a and col_b
        // are present across message + details.
        let all_messages: Vec<&str> = std::iter::once(message.as_str())
            .chain(details.iter().map(String::as_str))
            .collect();
        assert!(
            all_messages.iter().any(|m| m.contains("col_a"))
                && all_messages.iter().any(|m| m.contains("col_b")),
            "both columns should appear across message + details: message={message}, details={details:?}"
        );
        // Exactly one column is in message, the rest in details.
        assert_eq!(
            details.len(),
            1,
            "expected exactly one detail for two failing columns"
        );
    }
}

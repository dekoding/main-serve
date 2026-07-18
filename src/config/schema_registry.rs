/// Schema registry: loads JSON schemas from config, compiles them, and
/// provides per-column validators for the CRUD handlers.
///
/// The registry is built once at config load time and is shareable across
/// requests (via `Arc`).
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jsonschema::Validator;
use serde_json::Value;

use crate::config::types::schema::{GlobalSchema, JsonSchema, SchemaSource};
use crate::config::types::{ColumnConfig, ColumnType};
use crate::error::AppError;

/// Registry of compiled JSON schemas for column-level validation.
///
/// Built once at config load time and is shareable across requests (via `Arc`).
#[derive(Debug)]
pub struct SchemaRegistry {
    /// Compiled global schemas, keyed by name.
    global_schemas: HashMap<String, GlobalSchema>,
    /// Per-column validators, keyed by (table_name, column_name).
    /// Only populated for JSONB/columns that have a schema configured.
    column_validators: HashMap<(String, String), Arc<JsonSchema>>,
}

impl SchemaRegistry {
    /// Build a schema registry from config tables and global schemas.
    ///
    /// # Errors
    ///
    /// Returns `AppError::Config` if:
    /// - A global schema is not valid JSON Schema
    /// - An external schema file cannot be loaded or parsed
    /// - A `validation_schema_ref` references a non-existent global schema
    /// - A column with validation is not a JSONB/JSON type
    pub fn new(
        config: &crate::config::types::AppConfig,
        config_path: &Path,
    ) -> Result<Self, AppError> {
        // 1. Compile global schemas.
        let mut global_schemas = HashMap::new();
        if let Some(ref schemas) = config.global_schemas {
            for (name, raw) in schemas {
                let json_value = Self::yaml_to_json(raw)?;
                let compiled = Validator::new(&json_value).map_err(|e| {
                    AppError::Config(format!("Invalid global schema '{name}': {e}"))
                })?;
                global_schemas.insert(
                    name.clone(),
                    GlobalSchema::new(
                        name.clone(),
                        JsonSchema::new(compiled, SchemaSource::GlobalRef(name.clone())),
                    ),
                );
            }
        }

        // 2. Process tables and build per-column validators.
        let mut column_validators = HashMap::new();

        for table in &config.tables {
            for col in &table.columns {
                // Only JSONB/JSON columns support schema validation
                if !matches!(col.column_type, ColumnType::Jsonb | ColumnType::Json) {
                    continue;
                }

                let schema = Self::resolve_column_schema(col, &global_schemas, config_path)?;

                if let Some(schema) = schema {
                    column_validators
                        .insert((table.name.clone(), col.name.clone()), Arc::new(schema));
                }
            }
        }

        Ok(Self {
            global_schemas,
            column_validators,
        })
    }

    /// Resolve the schema for a column by examining its three validation fields.
    ///
    /// Returns `Some(JsonSchema)` if the column has a schema configured, or
    /// `None` if no validation is configured.
    ///
    /// If multiple validation fields are set, the first one found (in order:
    /// `validation_schema`, `validation`, `validation_schema_ref`) is used.
    fn resolve_column_schema(
        col: &ColumnConfig,
        global_schemas: &HashMap<String, GlobalSchema>,
        config_path: &Path,
    ) -> Result<Option<JsonSchema>, AppError> {
        // Prefer external file reference
        if let Some(ref path_str) = col.validation_schema {
            let path = PathBuf::from(path_str);
            let full_path = config_path.parent().unwrap_or(Path::new(".")).join(&path);
            let resolved = full_path.canonicalize().map_err(|e| {
                AppError::Config(format!(
                    "Failed to resolve external schema file '{}': {e}",
                    path.display()
                ))
            })?;
            let json_value = Self::load_schema_file(&resolved)?;
            let compiled = Validator::new(&json_value).map_err(|e| {
                AppError::Config(format!(
                    "Invalid schema in external file '{}' (column '{}'): {e}",
                    resolved.display(),
                    col.name
                ))
            })?;
            return Ok(Some(JsonSchema::new(
                compiled,
                SchemaSource::ExternalFile(resolved),
            )));
        }

        // Then prefer inline schema
        if let Some(ref inline) = col.validation {
            let json_value = Self::yaml_to_json(inline)?;
            let compiled = Validator::new(&json_value).map_err(|e| {
                AppError::Config(format!(
                    "Invalid inline schema for column '{}': {e}",
                    col.name
                ))
            })?;
            return Ok(Some(JsonSchema::new(compiled, SchemaSource::Inline)));
        }

        // Then prefer global schema reference
        if let Some(ref ref_str) = col.validation_schema_ref {
            // Accept both "name" and "global_schemas.name" formats
            let name = ref_str.strip_prefix("global_schemas.").unwrap_or(ref_str);
            let global = global_schemas.get(name).ok_or_else(|| {
                AppError::Config(format!(
                    "Global schema '{name}' not found (referenced by column '{}')",
                    col.name
                ))
            })?;
            return Ok(Some(global.schema().clone()));
        }

        Ok(None)
    }

    /// Load a JSON Schema file (YAML input -> JSON Value).
    fn load_schema_file(path: &Path) -> Result<Value, AppError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            AppError::Config(format!(
                "Failed to read schema file '{}': {e}",
                path.display()
            ))
        })?;
        let parsed: Value = serde_yaml::from_str(&content).map_err(|e| {
            AppError::Config(format!(
                "Failed to parse schema file '{}': {e}",
                path.display()
            ))
        })?;
        Ok(parsed)
    }

    /// Convert a YAML value to a JSON value for schema compilation.
    fn yaml_to_json(value: &serde_yaml::Value) -> Result<Value, AppError> {
        serde_json::to_value(value)
            .map_err(|e| AppError::Config(format!("Failed to convert schema value to JSON: {e}")))
    }

    /// Return the config for a specific column, if it has a validator.
    ///
    /// Only returns `Some` if the column is JSONB/JSON and has schema validation
    /// configured. Returns `None` for non-JSONB columns even if a validator
    /// somehow exists.
    pub fn get_column_validator(
        &self,
        table_name: &str,
        column_name: &str,
        column_type: &ColumnType,
    ) -> Option<Arc<JsonSchema>> {
        if !matches!(column_type, ColumnType::Jsonb | ColumnType::Json) {
            return None;
        }
        self.column_validators
            .get(&(table_name.to_string(), column_name.to_string()))
            .cloned()
    }

    /// Return the validators for all schema-validated columns in the given
    /// table, keyed by column name. Returns `None` if the table has no
    /// registered column validators.
    pub fn get_table_schema(&self, table_name: &str) -> Option<HashMap<String, Arc<JsonSchema>>> {
        let result: HashMap<String, Arc<JsonSchema>> = self
            .column_validators
            .iter()
            .filter_map(|((t, c), v)| {
                if t == table_name {
                    Some((c.clone(), v.clone()))
                } else {
                    None
                }
            })
            .collect();
        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Return an iterator over the names of all global schemas.
    pub fn global_schema_names(&self) -> impl Iterator<Item = &str> {
        self.global_schemas.keys().map(String::as_str)
    }
}

/// Validate a JSON value against a compiled schema, returning a list of human-readable
/// error messages.
///
/// If the value passes validation, returns an empty vec. Otherwise returns one
/// message per validation error.
pub fn validate_json_with_schema(value: &Value, schema: &JsonSchema) -> Vec<String> {
    let validator = schema.compiled();
    validator
        .iter_errors(value)
        .map(|e| {
            let path = e.instance_path().to_string();
            let message = e.to_string();
            if path.is_empty() {
                message
            } else {
                format!("{path}: {message}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::config::types::TableConfig;

    #[allow(clippy::type_complexity)] // test helper with 5-tuple column definition
    fn make_config_with_table_and_schema(
        schemas: &[(&str, &serde_yaml::Value)],
        columns: &[(
            &str,
            ColumnType,
            Option<&str>,
            Option<&serde_yaml::Value>,
            Option<&str>,
        )],
    ) -> crate::config::types::AppConfig {
        let mut global_schemas = HashMap::new();
        for (name, schema) in schemas {
            global_schemas.insert((*name).to_string(), (*schema).clone());
        }

        let table_columns: Vec<ColumnConfig> = columns
            .iter()
            .map(
                |(name, col_type, validation_schema, validation, validation_schema_ref)| {
                    ColumnConfig {
                        name: (*name).to_string(),
                        column_type: *col_type,
                        primary_key: false,
                        nullable: true,
                        default: None,
                        unique: false,
                        indexed: false,
                        validation_schema: validation_schema.map(String::from),
                        validation: validation.cloned(),
                        validation_schema_ref: validation_schema_ref.map(String::from),
                    }
                },
            )
            .collect();

        AppConfig {
            databases: {
                let mut m = HashMap::new();
                m.insert(
                    "main".to_string(),
                    crate::config::types::DatabaseConfig {
                        driver: crate::config::types::DatabaseDriver::Sqlite,
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
                columns: table_columns,
                foreign_keys: vec![],
            }],
            global_schemas: if global_schemas.is_empty() {
                None
            } else {
                Some(global_schemas)
            },
            ..Default::default()
        }
    }

    #[test]
    fn test_create_registry_no_global_schemas() {
        let config = make_config_with_table_and_schema(&[], &[]);
        let temp = tempfile::NamedTempFile::new().unwrap();
        let result = SchemaRegistry::new(&config, temp.path());
        assert!(result.is_ok(), "empty config should work");
    }

    #[test]
    fn test_create_registry_with_valid_global_schema() {
        let inline_schema: serde_yaml::Value = serde_yaml::from_str(
            r#"
type: object
required:
  - name
properties:
  name:
    type: string
"#,
        )
        .unwrap();

        let config = make_config_with_table_and_schema(&[("test_schema", &inline_schema)], &[]);
        let temp = tempfile::NamedTempFile::new().unwrap();
        let result = SchemaRegistry::new(&config, temp.path());
        assert!(result.is_ok(), "valid global schema should compile");
        let registry = result.unwrap();
        assert!(registry.global_schema_names().any(|n| n == "test_schema"));
    }

    #[test]
    fn test_create_registry_with_invalid_global_schema() {
        let inline_schema: serde_yaml::Value =
            serde_yaml::from_str(r#"type: "invalid_non_existent_type_xyz""#).unwrap();

        let config = make_config_with_table_and_schema(&[("bad_schema", &inline_schema)], &[]);
        let temp = tempfile::NamedTempFile::new().unwrap();
        let result = SchemaRegistry::new(&config, temp.path());
        assert!(
            result.is_err(),
            "invalid global schema should fail to compile"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Invalid global schema"),
            "Error should mention invalid schema, got: {err}"
        );
    }

    #[test]
    fn test_create_registry_with_external_file() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let schema_path = dir.path().join("schema.json");
        std::fs::write(
            &schema_path,
            r#"{"type": "object", "properties": {"name": {"type": "string"}}}
"#,
        )
        .unwrap();

        let config = make_config_with_table_and_schema(
            &[],
            &[("data", ColumnType::Jsonb, Some("schema.json"), None, None)],
        );

        // Create a temp config file in the same directory as the schema
        // so the relative path "schema.json" resolves correctly
        let temp = tempfile::NamedTempFile::new_in(dir.path()).unwrap();

        let result = SchemaRegistry::new(&config, temp.path());
        assert!(
            result.is_ok(),
            "external file should load: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_get_column_validator_finds_schema() {
        let inline_schema: serde_yaml::Value = serde_yaml::from_str("type: object").unwrap();

        let config = make_config_with_table_and_schema(
            &[],
            &[("data", ColumnType::Jsonb, None, Some(&inline_schema), None)],
        );
        let temp = tempfile::NamedTempFile::new().unwrap();
        let registry = SchemaRegistry::new(&config, temp.path()).unwrap();

        let validator = registry.get_column_validator("test_table", "data", &ColumnType::Jsonb);
        assert!(
            validator.is_some(),
            "should find validator for column with inline schema"
        );
    }

    #[test]
    fn test_get_column_validator_returns_none_without_schema() {
        let config =
            make_config_with_table_and_schema(&[], &[("name", ColumnType::Text, None, None, None)]);
        let temp = tempfile::NamedTempFile::new().unwrap();
        let registry = SchemaRegistry::new(&config, temp.path()).unwrap();

        let validator = registry.get_column_validator("test_table", "name", &ColumnType::Text);
        assert!(
            validator.is_none(),
            "non-JSONB column should have no validator"
        );
    }

    #[test]
    fn test_create_registry_with_global_ref() {
        let inline_schema: serde_yaml::Value = serde_yaml::from_str(
            r#"
type: object
required:
  - name
properties:
  name:
    type: string
"#,
        )
        .unwrap();

        let config = make_config_with_table_and_schema(
            &[("global_ref_schema", &inline_schema)],
            &[(
                "data",
                ColumnType::Jsonb,
                None,
                None,
                Some("global_ref_schema"),
            )],
        );
        let temp = tempfile::NamedTempFile::new().unwrap();
        let registry = SchemaRegistry::new(&config, temp.path()).unwrap();

        let validator = registry.get_column_validator("test_table", "data", &ColumnType::Jsonb);
        assert!(
            validator.is_some(),
            "global ref should resolve to a validator"
        );
    }

    #[test]
    fn test_create_registry_with_external_file_missing() {
        let config = make_config_with_table_and_schema(
            &[],
            &[(
                "data",
                ColumnType::Jsonb,
                Some("no_such_file.json"),
                None,
                None,
            )],
        );
        let temp = tempfile::NamedTempFile::new().unwrap();
        let result = SchemaRegistry::new(&config, temp.path());
        assert!(result.is_err(), "missing external file should fail");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Failed to resolve") || err.contains("no_such_file"),
            "Error should mention the missing file, got: {err}"
        );
    }

    #[test]
    fn test_get_table_schema_names() {
        let inline: serde_yaml::Value = serde_yaml::from_str("type: object").unwrap();
        let inline2: serde_yaml::Value = serde_yaml::from_str(
            r#"
type: object
properties:
  key: { type: string }
"#,
        )
        .unwrap();

        let config = make_config_with_table_and_schema(
            &[],
            &[
                ("data", ColumnType::Jsonb, None, Some(&inline), None),
                ("metadata", ColumnType::Jsonb, None, Some(&inline2), None),
                ("name", ColumnType::Text, None, None, None),
            ],
        );
        let temp = tempfile::NamedTempFile::new().unwrap();
        let registry = SchemaRegistry::new(&config, temp.path()).unwrap();

        let schemas = registry.get_table_schema("test_table");
        assert!(schemas.is_some(), "table should have schema entry");
        let schemas = schemas.unwrap();
        assert_eq!(schemas.len(), 2);
        assert!(schemas.contains_key("data"));
        assert!(schemas.contains_key("metadata"));
    }

    #[test]
    fn test_validate_json_with_schema_valid() {
        let schema: serde_yaml::Value = serde_yaml::from_str(
            r#"
type: object
required:
  - name
properties:
  name:
    type: string
"#,
        )
        .unwrap();
        let validator = Validator::new(&serde_json::to_value(&schema).unwrap()).unwrap();
        let json_schema = JsonSchema::new(validator, SchemaSource::Inline);

        let data: serde_yaml::Value = serde_yaml::from_str("name: Alice\nage: 30").unwrap();
        let json_data = serde_json::to_value(&data).unwrap();

        let errors = validate_json_with_schema(&json_data, &json_schema);
        assert!(
            errors.is_empty(),
            "valid data should produce no errors: {errors:?}"
        );
    }

    #[test]
    fn test_validate_json_with_schema_invalid_type() {
        let schema: serde_yaml::Value = serde_yaml::from_str(
            r#"
type: object
required:
  - name
properties:
  name:
    type: string
"#,
        )
        .unwrap();
        let validator = Validator::new(&serde_json::to_value(&schema).unwrap()).unwrap();
        let json_schema = JsonSchema::new(validator, SchemaSource::Inline);

        let invalid_data = serde_json::json!("not an object");
        let errors = validate_json_with_schema(&invalid_data, &json_schema);
        assert!(
            !errors.is_empty(),
            "invalid data should produce errors: {errors:?}"
        );

        let error_str = errors.join(", ");
        assert!(
            error_str.contains("type") || error_str.contains("string"),
            "Error should mention type, got: {error_str}"
        );
    }

    #[test]
    fn test_validate_json_with_schema_missing_required() {
        let schema: serde_yaml::Value = serde_yaml::from_str(
            r#"
type: object
required:
  - name
  - email
properties:
  name:
    type: string
  email:
    type: string
"#,
        )
        .unwrap();
        let validator = Validator::new(&serde_json::to_value(&schema).unwrap()).unwrap();
        let json_schema = JsonSchema::new(validator, SchemaSource::Inline);

        let data = serde_json::json!({"name": "Alice"});
        let errors = validate_json_with_schema(&data, &json_schema);
        assert!(
            !errors.is_empty(),
            "missing required field should produce errors: {errors:?}"
        );

        let error_str = errors.join(", ");
        assert!(
            error_str.contains("email"),
            "Error should mention missing 'email', got: {error_str}"
        );
    }
}

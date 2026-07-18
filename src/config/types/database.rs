/// Database connection and table schema configuration.
use serde::{Deserialize, Serialize};

/// A named database connection configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// DatabaseConfig
pub struct DatabaseConfig {
    /// Database backend to use.
    pub driver: DatabaseDriver,
    /// Connection URL (e.g. `sqlite://data.db` or `postgres://...`).
    pub url: String,
    /// Minimum number of connections in the pool.
    pub min_connections: u32,
    /// Maximum number of connections in the pool.
    pub max_connections: u32,
    /// Whether to run auto-migrations on startup and reload.
    pub auto_migrate: bool,
    /// When `true`, auto-migration may drop columns that exist in the database
    /// but are no longer present in the YAML config. When `false` (default),
    /// removed columns are only logged as warnings and left untouched.
    pub allow_destructive: bool,
    /// Pool acquire timeout in seconds.
    #[serde(default = "default_acquire_timeout")]
    pub acquire_timeout: u64,
}

/// item
fn default_acquire_timeout() -> u64 {
    5
}

impl Default for DatabaseConfig {
    /// item
    fn default() -> Self {
        Self {
            driver: DatabaseDriver::Sqlite,
            url: String::new(),
            min_connections: 1,
            max_connections: 10,
            auto_migrate: true,
            allow_destructive: false,
            acquire_timeout: default_acquire_timeout(),
        }
    }
}

/// Supported database backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
/// DatabaseDriver
pub enum DatabaseDriver {
    Postgres,
    Mysql,
    Sqlite,
}

/// Schema for a single database table (used for migrations and query building).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
/// TableConfig
pub struct TableConfig {
    /// Name of the table as it appears in the database.
    pub name: String,
    /// Name of the database this table belongs to.
    pub database: String,
    /// Column definitions.
    #[serde(default)]
    pub columns: Vec<ColumnConfig>,
    /// Foreign key constraints.
    #[serde(default)]
    pub foreign_keys: Vec<ForeignKeyConfig>,
}

/// A column definition within a table.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default, rename_all = "snake_case", deny_unknown_fields)]
/// ColumnConfig
pub struct ColumnConfig {
    /// Column name.
    pub name: String,
    /// SQL data type.
    #[serde(rename = "type")]
    pub column_type: ColumnType,
    /// Whether this column is part of the primary key.
    pub primary_key: bool,
    /// Whether this column allows NULL values.
    pub nullable: bool,
    /// Default value expression (raw SQL).
    pub default: Option<String>,
    /// Whether this column has a UNIQUE constraint.
    pub unique: bool,
    /// Whether to create an index on this column.
    pub indexed: bool,
    /// Path to an external JSON Schema file for validating this column's JSONB value.
    pub validation_schema: Option<String>,
    /// Inline JSON Schema for validating this column's JSONB value.
    pub validation: Option<serde_yaml::Value>,
    /// Reference to a named global schema.
    pub validation_schema_ref: Option<String>,
}

impl Default for ColumnConfig {
    /// item
    fn default() -> Self {
        Self {
            name: String::new(),
            column_type: ColumnType::Text,
            primary_key: false,
            nullable: true,
            default: None,
            unique: false,
            indexed: false,
            validation_schema: None,
            validation: None,
            validation_schema_ref: None,
        }
    }
}

/// Supported column data types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
/// ColumnType
pub enum ColumnType {
    Integer,
    Bigint,
    Smallint,
    Serial,
    Bigserial,
    Text,
    Varchar,
    Char,
    Boolean,
    Float,
    Double,
    Decimal,
    Date,
    Timestamp,
    Timestamptz,
    Uuid,
    Json,
    Jsonb,
    Blob,
    Bytea,
}

impl std::fmt::Display for ColumnType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ColumnType::Integer => write!(f, "integer"),
            ColumnType::Bigint => write!(f, "bigint"),
            ColumnType::Smallint => write!(f, "smallint"),
            ColumnType::Serial => write!(f, "serial"),
            ColumnType::Bigserial => write!(f, "bigserial"),
            ColumnType::Text => write!(f, "text"),
            ColumnType::Varchar => write!(f, "varchar"),
            ColumnType::Char => write!(f, "char"),
            ColumnType::Boolean => write!(f, "boolean"),
            ColumnType::Float => write!(f, "float"),
            ColumnType::Double => write!(f, "double"),
            ColumnType::Decimal => write!(f, "decimal"),
            ColumnType::Date => write!(f, "date"),
            ColumnType::Timestamp => write!(f, "timestamp"),
            ColumnType::Timestamptz => write!(f, "timestamptz"),
            ColumnType::Uuid => write!(f, "uuid"),
            ColumnType::Json => write!(f, "json"),
            ColumnType::Jsonb => write!(f, "jsonb"),
            ColumnType::Blob => write!(f, "blob"),
            ColumnType::Bytea => write!(f, "bytea"),
        }
    }
}

/// A foreign key constraint on a table.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
/// ForeignKeyConfig
pub struct ForeignKeyConfig {
    /// Column in this table that holds the foreign key.
    pub column: String,
    /// Target table name.
    pub references_table: String,
    /// Target column name.
    pub references_column: String,
    /// Action on parent row deletion.
    #[serde(default = "default_fk_action")]
    pub on_delete: ForeignKeyAction,
    /// Action on parent row update.
    #[serde(default = "default_fk_action")]
    pub on_update: ForeignKeyAction,
}

/// item
fn default_fk_action() -> ForeignKeyAction {
    ForeignKeyAction::Restrict
}

/// Foreign key referential actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
/// ForeignKeyAction
pub enum ForeignKeyAction {
    Cascade,
    SetNull,
    Restrict,
    NoAction,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_column_config_serialization_with_validation_schema() {
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
        let serialized = serde_yaml::to_string(&col).unwrap();
        let deserialized: ColumnConfig = serde_yaml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.name, "metadata");
        assert_eq!(
            deserialized.validation_schema.as_deref(),
            Some("./schemas/meta.json")
        );
        assert!(deserialized.validation.is_none());
        assert!(deserialized.validation_schema_ref.is_none());
    }

    #[test]
    fn test_column_config_serialization_with_inline_validation() {
        let inline_schema = serde_yaml::from_value(serde_yaml::Value::Mapping({
            let mut m = serde_yaml::Mapping::new();
            m.insert(
                serde_yaml::Value::String("type".into()),
                serde_yaml::Value::String("object".into()),
            );
            m
        }))
        .unwrap();
        let col = ColumnConfig {
            name: "data".to_string(),
            column_type: ColumnType::Jsonb,
            primary_key: false,
            nullable: true,
            default: None,
            unique: false,
            indexed: false,
            validation_schema: None,
            validation: Some(inline_schema),
            validation_schema_ref: None,
        };
        let serialized = serde_yaml::to_string(&col).unwrap();
        let deserialized: ColumnConfig = serde_yaml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.name, "data");
        assert!(deserialized.validation.is_some());
        assert!(deserialized.validation_schema.is_none());
        assert!(deserialized.validation_schema_ref.is_none());
    }

    #[test]
    fn test_column_config_serialization_with_global_ref() {
        let col = ColumnConfig {
            name: "content".to_string(),
            column_type: ColumnType::Json,
            primary_key: false,
            nullable: true,
            default: None,
            unique: false,
            indexed: false,
            validation_schema: None,
            validation: None,
            validation_schema_ref: Some("global_schemas.blog_post".to_string()),
        };
        let serialized = serde_yaml::to_string(&col).unwrap();
        let deserialized: ColumnConfig = serde_yaml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.name, "content");
        assert_eq!(deserialized.column_type, ColumnType::Json);
        assert_eq!(
            deserialized.validation_schema_ref.as_deref(),
            Some("global_schemas.blog_post")
        );
        assert!(deserialized.validation_schema.is_none());
        assert!(deserialized.validation.is_none());
    }

    #[test]
    fn test_column_config_deserializes_no_validation() {
        let yaml = r#"
name: "email"
type: "varchar"
nullable: false
"#;
        let col: ColumnConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(col.name, "email");
        assert_eq!(col.column_type, ColumnType::Varchar);
        assert!(!col.nullable);
        assert!(col.validation_schema.is_none());
        assert!(col.validation.is_none());
        assert!(col.validation_schema_ref.is_none());
    }

    #[test]
    fn test_column_type_display() {
        assert_eq!(ColumnType::Jsonb.to_string(), "jsonb");
        assert_eq!(ColumnType::Json.to_string(), "json");
        assert_eq!(ColumnType::Text.to_string(), "text");
        assert_eq!(ColumnType::Varchar.to_string(), "varchar");
    }
}

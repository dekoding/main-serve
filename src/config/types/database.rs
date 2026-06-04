/// Database connection and table schema configuration.
use serde::Deserialize;

/// A named database connection configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
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

fn default_acquire_timeout() -> u64 {
    5
}

impl Default for DatabaseConfig {
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
pub enum DatabaseDriver {
    Postgres,
    Mysql,
    Sqlite,
}

/// Schema for a single database table (used for migrations and query building).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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

/// A unique identifier for a table (name + database combination).
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct TableIdentifier {
    pub name: String,
    pub database: String,
}

impl TableIdentifier {
    /// Create a new table identifier from a table name and database name.
    pub fn new(name: impl Into<String>, database: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            database: database.into(),
        }
    }
}

/// A column definition within a table.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
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
}

impl Default for ColumnConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            column_type: ColumnType::Text,
            primary_key: false,
            nullable: true,
            default: None,
            unique: false,
            indexed: false,
        }
    }
}

/// Supported column data types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
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

/// A foreign key constraint on a table.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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

fn default_fk_action() -> ForeignKeyAction {
    ForeignKeyAction::Restrict
}

/// Foreign key referential actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForeignKeyAction {
    Cascade,
    SetNull,
    Restrict,
    NoAction,
}

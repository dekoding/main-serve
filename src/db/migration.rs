/// Auto-migration: translates YAML table schemas into SQL DDL.
///
/// On each migration run the system introspects existing tables and computes
/// a diff against the YAML-defined schema:
///
/// * **New tables** -> `CREATE TABLE`
/// * **New columns** (in YAML but not in DB) -> `ALTER TABLE ... ADD COLUMN`
/// * **Removed columns** (in DB but not in YAML) -> `ALTER TABLE ... DROP COLUMN`
///   only when `allow_destructive` is `true` for that database; otherwise a
///   warning is logged and the column is left untouched.
/// * **Type / constraint mismatches** -> logged as warnings; no automatic
///   alteration is attempted because cross-driver support is inconsistent.
///
/// Indexes are created idempotently. `SQLite` and `PostgreSQL` use
/// `CREATE INDEX IF NOT EXISTS`; `MySQL` introspects existing indexes first and
/// only creates missing ones.
use std::collections::HashMap;
use std::fmt::Write;

use crate::config::types::{
    ColumnConfig, ColumnType, DatabaseConfig, DatabaseDriver, ForeignKeyAction, TableConfig,
};
use crate::db::pool::DatabasePool;
use crate::db::query::helpers::quote_identifier;
use crate::error::AppError;

/// Create the JWT token revocation table if it does not exist.
///
/// The table has columns:
///   - `jti` varchar(255) PRIMARY KEY
///   - `revoked_at` timestamptz NOT NULL
///   - `expires_at` timestamptz NOT NULL
///
/// # Errors
///
/// Returns `AppError::Database` if the CREATE TABLE statement fails.
pub async fn create_revocation_tables<S: ::std::hash::BuildHasher + Sync>(
    pools: &HashMap<String, DatabasePool, S>,
) -> Result<(), AppError> {
    let sql = match pools.values().next().map(super::pool::DatabasePool::driver) {
        Some(DatabaseDriver::Sqlite) => {
            "CREATE TABLE IF NOT EXISTS \"token_blacklist\" (\
             \"jti\" TEXT PRIMARY KEY NOT NULL, \
             \"revoked_at\" TEXT NOT NULL, \
             \"expires_at\" TEXT NOT NULL)"
        }
        Some(DatabaseDriver::Postgres) => {
            "CREATE TABLE IF NOT EXISTS \"token_blacklist\" (\
             \"jti\" VARCHAR(255) PRIMARY KEY NOT NULL, \
             \"revoked_at\" TIMESTAMPTZ NOT NULL, \
             \"expires_at\" TIMESTAMPTZ NOT NULL)"
        }
        Some(DatabaseDriver::Mysql) => {
            "CREATE TABLE IF NOT EXISTS `token_blacklist` (\
             `jti` VARCHAR(255) PRIMARY KEY NOT NULL, \
             `revoked_at` DATETIME NOT NULL, \
             `expires_at` DATETIME NOT NULL)"
        }
        None => return Ok(()),
    };

    // Create the table in all pools (should be the same database).
    for (name, pool) in pools {
        tracing::debug!("Ensuring revocation table exists in database '{name}'");
        tracing::debug!("Revocation table DDL: {sql}");
        pool.execute_raw(sql).await.map_err(|e| {
            AppError::ConfigurationError(format!(
                "Failed to create revocation table in database '{name}': {e}"
            ))
        })?;
    }

    Ok(())
}

/// Run auto-migrations for all tables that belong to databases with `auto_migrate: true`.
///
/// For each table the function checks whether it already exists in the database.
/// * If it does **not** exist, the full `CREATE TABLE` DDL is executed.
/// * If it **does** exist, existing columns are introspected and compared
///   against the YAML config to produce `ALTER TABLE ... ADD COLUMN` and,
///   when `allow_destructive` is enabled, `ALTER TABLE ... DROP COLUMN` statements.
///
/// Tables are sorted topologically so that tables referenced by foreign keys
/// are created first, which is required by Postgres and `MySQL`.
///
/// # Errors
///
/// Returns `AppError::Database` if any SQL statement fails, or `AppError::Internal`
/// if a referenced database pool is missing.
///
/// The `implicit_hasher` allow is needed because the function uses `HashMap::get()`
/// on both `pools` and `databases` to look up entries by string key, which invokes
/// the default `DefaultHasher`. Using an explicit `RandomState` in the parameter
/// types would be verbose without adding safety (the keys are strings, not secrets).
#[allow(clippy::implicit_hasher)] // uses HashMap::get() on pools and databases
pub async fn run_migrations(
    tables: &[TableConfig],
    pools: &HashMap<String, DatabasePool>,
    databases: &HashMap<String, DatabaseConfig>,
) -> Result<(), AppError> {
    // Sort tables topologically: tables referenced by foreign keys must come first.
    let sorted = sort_tables_topologically(tables)?;

    for table_config in &sorted {
        let table_name = &table_config.name;
        let db_name = &table_config.database;

        let db_config = databases.get(db_name);

        // Check if auto_migrate is enabled for this database.
        let should_migrate = db_config.is_none_or(|db| db.auto_migrate);
        if !should_migrate {
            tracing::debug!(
                "Skipping migration for table '{table_name}' (auto_migrate disabled for '{db_name}')"
            );
            continue;
        }

        let pool = pools.get(db_name).ok_or_else(|| {
            AppError::ConfigurationError(format!(
                "Table '{table_name}' references database '{db_name}' which has no pool"
            ))
        })?;

        let driver = pool.driver();

        // Introspect existing columns for this table.
        let existing_columns = get_existing_columns(pool, table_name).await?;

        if existing_columns.is_empty() {
            // Table does not exist (or has no columns) - create it.
            let sql = generate_create_table(table_config, driver);
            tracing::info!("Creating table '{table_name}' on database '{db_name}'");
            tracing::debug!("DDL: {sql}");

            pool.execute_raw(&sql).await.map_err(|e| {
                AppError::ConfigurationError(format!(
                    "Migration failed for table '{table_name}': {e}"
                ))
            })?;
        } else {
            // Table exists - compute diff and apply ALTER TABLE statements.
            let allow_destructive = db_config.is_some_and(|db| db.allow_destructive);
            alter_existing_table(pool, table_config, &existing_columns, allow_destructive).await?;
        }

        // Create indexes idempotently.
        let existing_indexes = if driver == DatabaseDriver::Mysql {
            get_existing_indexes(pool, table_name).await?
        } else {
            std::collections::HashSet::new()
        };

        for col in &table_config.columns {
            if col.indexed && !col.primary_key {
                let idx_name = format!("idx_{table_name}_{}", col.name);
                if driver == DatabaseDriver::Mysql && existing_indexes.contains(&idx_name) {
                    tracing::debug!(
                        "Skipping index creation for '{idx_name}' on '{table_name}' (already exists)"
                    );
                    continue;
                }

                let idx_sql = generate_create_index(table_name, &col.name, driver);
                tracing::debug!("Index DDL: {idx_sql}");
                pool.execute_raw(&idx_sql).await.map_err(|e| {
                    AppError::ConfigurationError(format!(
                        "Failed to create index on '{table_name}.{}': {e}",
                        col.name
                    ))
                })?;
            }
        }
    }

    Ok(())
}

// =============================================================================
// Schema introspection
// =============================================================================

/// A column as it currently exists in the database.
#[derive(Debug)]
/// Represents a column currently existing in the database, tracked by name only.
struct ExistingColumn {
    name: String,
}

/// Query the database for the columns of `table_name`.
///
/// Returns an empty `Vec` when the table does not exist (the introspection
/// queries return no rows for non-existent tables).
async fn get_existing_columns(
    pool: &DatabasePool,
    table_name: &str,
) -> Result<Vec<ExistingColumn>, AppError> {
    let rows = match pool {
        DatabasePool::Sqlite(_) => {
            let sql = format!(
                "PRAGMA table_info({})",
                quote_identifier(table_name, pool.driver())
            );
            pool.fetch_all_json(&sql, &[]).await?
        }
        DatabasePool::Postgres(_) => {
            let sql = "SELECT column_name FROM information_schema.columns \
                        WHERE table_name = $1 ORDER BY ordinal_position";
            pool.fetch_all_json(sql, &[serde_json::Value::String(table_name.to_owned())])
                .await?
        }
        DatabasePool::Mysql(_) => {
            let sql = "SELECT column_name FROM information_schema.columns \
                        WHERE table_schema = DATABASE() AND table_name = ? \
                        ORDER BY ordinal_position";
            pool.fetch_all_json(sql, &[serde_json::Value::String(table_name.to_owned())])
                .await?
        }
    };

    let columns = rows
        .iter()
        .filter_map(|row| {
            // SQLite PRAGMA returns "name"; information_schema returns "column_name".
            row.get("name")
                .or_else(|| row.get("column_name"))
                .or_else(|| row.get("COLUMN_NAME"))
                .and_then(|v| v.as_str())
                .map(|s| ExistingColumn { name: s.to_owned() })
        })
        .collect();

    Ok(columns)
}

/// Query the database for the indexes of `table_name`.
async fn get_existing_indexes(
    pool: &DatabasePool,
    table_name: &str,
) -> Result<std::collections::HashSet<String>, AppError> {
    let rows = match pool {
        DatabasePool::Sqlite(_) => {
            let sql = format!(
                "PRAGMA index_list({})",
                quote_identifier(table_name, pool.driver())
            );
            pool.fetch_all_json(&sql, &[]).await?
        }
        DatabasePool::Postgres(_) => {
            let sql = "SELECT indexname FROM pg_indexes WHERE tablename = $1";
            pool.fetch_all_json(sql, &[serde_json::Value::String(table_name.to_owned())])
                .await?
        }
        DatabasePool::Mysql(_) => {
            let sql = "SELECT index_name FROM information_schema.statistics \
                        WHERE table_schema = DATABASE() AND table_name = ? \
                        GROUP BY index_name";
            pool.fetch_all_json(sql, &[serde_json::Value::String(table_name.to_owned())])
                .await?
        }
    };

    Ok(rows
        .iter()
        .filter_map(|row| {
            row.get("name")
                .or_else(|| row.get("indexname"))
                .or_else(|| row.get("index_name"))
                .or_else(|| row.get("INDEX_NAME"))
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .collect())
}

// =============================================================================
// ALTER TABLE logic
// =============================================================================

/// Compare the YAML-defined columns against the existing database columns and
/// apply the necessary `ALTER TABLE` statements.
async fn alter_existing_table(
    pool: &DatabasePool,
    table_config: &TableConfig,
    existing_columns: &[ExistingColumn],
    allow_destructive: bool,
) -> Result<(), AppError> {
    let driver = pool.driver();
    let table_name = &table_config.name;
    let existing_names: std::collections::HashSet<String> =
        existing_columns.iter().map(|c| c.name.clone()).collect();

    let yaml_names: std::collections::HashSet<&str> = table_config
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .collect();

    // --- ADD columns that are in YAML but not in DB --------------------------
    for col in &table_config.columns {
        if existing_names.contains(&col.name) {
            continue;
        }

        // Safety: NOT NULL without DEFAULT on a non-empty table will fail.
        // SQLite additionally forbids PRIMARY KEY and UNIQUE in ADD COLUMN.
        if col.primary_key {
            tracing::warn!(
                "Cannot add primary key column '{}' to existing table '{table_name}' \
                 via ALTER TABLE - skipping. A manual migration is required.",
                col.name
            );
            continue;
        }

        if !col.nullable && col.default.is_none() {
            tracing::warn!(
                "Cannot add NOT NULL column '{}' without a DEFAULT to existing \
                 table '{table_name}' - skipping. Either set `nullable: true` or \
                 provide a `default` value.",
                col.name
            );
            continue;
        }

        let sql = generate_add_column(table_name, col, driver);
        tracing::info!("Adding column '{}' to table '{table_name}'", col.name);
        tracing::debug!("DDL: {sql}");

        pool.execute_raw(&sql).await.map_err(|e| {
            AppError::ConfigurationError(format!(
                "Failed to add column '{}' to table '{table_name}': {e}",
                col.name
            ))
        })?;
    }

    // --- DROP columns that are in DB but not in YAML -------------------------
    for existing in existing_columns {
        if yaml_names.contains(existing.name.as_str()) {
            continue;
        }

        if allow_destructive {
            let sql = generate_drop_column(table_name, &existing.name, driver);
            tracing::info!(
                "Dropping column '{}' from table '{table_name}' (allow_destructive is on)",
                existing.name
            );
            tracing::debug!("DDL: {sql}");

            pool.execute_raw(&sql).await.map_err(|e| {
                AppError::ConfigurationError(format!(
                    "Failed to drop column '{}' from table '{table_name}': {e}",
                    existing.name
                ))
            })?;
        } else {
            tracing::warn!(
                "Column '{}' exists in table '{table_name}' but is not in the YAML config. \
                 Set `allow_destructive: true` on the database to drop it automatically.",
                existing.name
            );
        }
    }

    Ok(())
}

/// Generate an ALTER TABLE ... ADD COLUMN statement for a single column.
#[must_use]
/// Generates an ALTER TABLE ... ADD COLUMN SQL statement for the given column.
fn generate_add_column(table_name: &str, col: &ColumnConfig, driver: DatabaseDriver) -> String {
    let mut col_def = format!(
        "{} {}",
        quote_identifier(&col.name, driver),
        column_type_to_sql(col.column_type, driver),
    );

    if !col.nullable {
        col_def.push_str(" NOT NULL");
    }

    // SQLite does not support UNIQUE in ADD COLUMN; Postgres & MySQL do.
    if col.unique && driver != DatabaseDriver::Sqlite {
        col_def.push_str(" UNIQUE");
    }

    if let Some(ref default) = col.default {
        let _ = write!(col_def, " DEFAULT {default}");
    }

    format!(
        "ALTER TABLE {} ADD COLUMN {col_def}",
        quote_identifier(table_name, driver)
    )
}

/// Generate an ALTER TABLE ... DROP COLUMN statement.
#[must_use]
/// Generates an ALTER TABLE ... DROP COLUMN SQL statement.
fn generate_drop_column(table_name: &str, column_name: &str, driver: DatabaseDriver) -> String {
    let col = quote_identifier(column_name, driver);
    format!(
        "ALTER TABLE {} DROP COLUMN {col}",
        quote_identifier(table_name, driver)
    )
}

/// Generate a CREATE TABLE IF NOT EXISTS statement.
#[must_use]
/// Generates a CREATE TABLE IF NOT EXISTS SQL statement from a `TableConfig`.
fn generate_create_table(table: &TableConfig, driver: DatabaseDriver) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Column definitions.
    for col in &table.columns {
        let col_def = generate_column_def(col, driver);
        parts.push(col_def);
        if col.primary_key {
            // Track PK columns for constraint generation.
        }
    }

    // Primary key constraint.
    add_pk_constraint(&mut parts, &table.columns, driver);

    // Foreign key definitions.
    for fk in &table.foreign_keys {
        parts.push(format!(
            "  FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {} ON UPDATE {}",
            quote_identifier(&fk.column, driver),
            quote_identifier(&fk.references_table, driver),
            quote_identifier(&fk.references_column, driver),
            fk_action_to_sql(fk.on_delete),
            fk_action_to_sql(fk.on_update),
        ));
    }

    format!(
        "CREATE TABLE IF NOT EXISTS {} (\n{}\n)",
        quote_identifier(&table.name, driver),
        parts.join(",\n")
    )
}

/// Generate a single column definition DDL fragment.
fn generate_column_def(col: &ColumnConfig, driver: DatabaseDriver) -> String {
    let mut col_def = format!(
        "  {} {}",
        quote_identifier(&col.name, driver),
        column_type_to_sql(col.column_type, driver)
    );

    if col.primary_key && driver == DatabaseDriver::Sqlite {
        if !col_def.contains("PRIMARY KEY") {
            col_def.push_str(" PRIMARY KEY");
        }
        if !col_def.contains("AUTOINCREMENT") {
            col_def.push_str(" AUTOINCREMENT");
        }
    }

    if !col.nullable || col.primary_key {
        col_def.push_str(" NOT NULL");
    }

    if col.unique {
        col_def.push_str(" UNIQUE");
    }

    if let Some(ref default) = col.default {
        let _ = write!(col_def, " DEFAULT {default}");
    }

    col_def
}

/// Add the primary key constraint to the parts list.
fn add_pk_constraint(parts: &mut Vec<String>, columns: &[ColumnConfig], driver: DatabaseDriver) {
    let pk_columns: Vec<String> = columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| quote_identifier(&c.name, driver))
        .collect();

    if pk_columns.is_empty() {
        return;
    }

    let is_single_pk = pk_columns.len() == 1;
    let should_add_constraint = if driver == DatabaseDriver::Sqlite {
        !is_single_pk
    } else {
        true
    };

    if should_add_constraint {
        parts.push(format!("  PRIMARY KEY ({})", pk_columns.join(", ")));
    }
}

/// Generate an index creation statement.
#[must_use]
/// Generates a CREATE INDEX (or CREATE INDEX IF NOT EXISTS) SQL statement.
fn generate_create_index(table_name: &str, column_name: &str, driver: DatabaseDriver) -> String {
    let idx_name = format!("idx_{table_name}_{column_name}");
    match driver {
        DatabaseDriver::Mysql => format!(
            "CREATE INDEX {} ON {} ({})",
            quote_identifier(&idx_name, driver),
            quote_identifier(table_name, driver),
            quote_identifier(column_name, driver),
        ),
        DatabaseDriver::Sqlite | DatabaseDriver::Postgres => format!(
            "CREATE INDEX IF NOT EXISTS {} ON {} ({})",
            quote_identifier(&idx_name, driver),
            quote_identifier(table_name, driver),
            quote_identifier(column_name, driver),
        ),
    }
}

/// Map a `ColumnType` to its SQL type string for the given driver.
const fn column_type_to_sql(ct: ColumnType, driver: DatabaseDriver) -> &'static str {
    match (ct, driver) {
        // Integer types
        (ColumnType::Integer, _)
        | (ColumnType::Serial | ColumnType::Boolean, DatabaseDriver::Sqlite) => "INTEGER",
        (ColumnType::Bigint, _) => "BIGINT",
        (ColumnType::Smallint, _) => "SMALLINT",
        (ColumnType::Serial, DatabaseDriver::Postgres) => "SERIAL",
        (ColumnType::Serial, DatabaseDriver::Mysql) => "INTEGER AUTO_INCREMENT",
        (ColumnType::Bigserial, DatabaseDriver::Sqlite) => "INTEGER PRIMARY KEY AUTOINCREMENT",
        (ColumnType::Bigserial, DatabaseDriver::Postgres) => "BIGSERIAL",
        (ColumnType::Bigserial, DatabaseDriver::Mysql) => "BIGINT AUTO_INCREMENT",

        // Text types
        (ColumnType::Text, _)
        | (
            ColumnType::Varchar
            | ColumnType::Char
            | ColumnType::Date
            | ColumnType::Timestamp
            | ColumnType::Timestamptz
            | ColumnType::Uuid,
            DatabaseDriver::Sqlite,
        ) => "TEXT",
        (ColumnType::Varchar, _) => "VARCHAR(255)",
        (ColumnType::Char, _) => "CHAR(255)",

        // Boolean
        (ColumnType::Boolean, _) => "BOOLEAN",

        // Floating point
        (ColumnType::Float, _)
        | (ColumnType::Double | ColumnType::Decimal, DatabaseDriver::Sqlite) => "REAL",
        (ColumnType::Double, _) => "DOUBLE PRECISION",
        (ColumnType::Decimal, _) => "DECIMAL(10,2)",

        // Date/Time
        (ColumnType::Date, _)
        | (ColumnType::Timestamp | ColumnType::Timestamptz, DatabaseDriver::Mysql) => "DATETIME",
        (ColumnType::Timestamp, DatabaseDriver::Postgres) => "TIMESTAMP",
        (ColumnType::Timestamptz, DatabaseDriver::Postgres) => "TIMESTAMPTZ",

        // UUID
        (ColumnType::Uuid, DatabaseDriver::Postgres) => "UUID",
        (ColumnType::Uuid, DatabaseDriver::Mysql) => "CHAR(36)",

        // JSON
        (ColumnType::Json, _) | (ColumnType::Jsonb, DatabaseDriver::Mysql) => "JSON",
        (ColumnType::Jsonb, DatabaseDriver::Sqlite | DatabaseDriver::Postgres) => "JSONB",

        // Binary
        (ColumnType::Blob | ColumnType::Bytea, DatabaseDriver::Mysql | DatabaseDriver::Sqlite) => {
            "BLOB"
        }
        (ColumnType::Blob | ColumnType::Bytea, DatabaseDriver::Postgres) => "BYTEA",
    }
}

/// Map a `ForeignKeyAction` to its SQL fragment.
const fn fk_action_to_sql(action: ForeignKeyAction) -> &'static str {
    match action {
        ForeignKeyAction::Cascade => "CASCADE",
        ForeignKeyAction::SetNull => "SET NULL",
        ForeignKeyAction::Restrict => "RESTRICT",
        ForeignKeyAction::NoAction => "NO ACTION",
    }
}

/// Sort tables topologically so that referenced tables come before referencing tables.
///
/// This ensures foreign key constraints are valid when CREATE TABLE is executed,
/// which is required by Postgres and `MySQL` (`SQLite` ignores FK constraints by default).
///
/// Uses Kahn's algorithm with `VecDeque` for O(n) queue operations instead of
/// `Vec::remove(0)` which is O(n) per dequeue, resulting in O(n^2) total.
///
/// # Errors
///
/// Returns `AppError::Internal` if a cycle is detected in the dependency graph
/// (i.e., two or more tables have circular foreign key references).
fn sort_tables_topologically(tables: &[TableConfig]) -> Result<Vec<TableConfig>, AppError> {
    // Build a map of table_name -> index
    let table_map: std::collections::HashMap<&str, usize> = tables
        .iter()
        .enumerate()
        .map(|(i, t)| (t.name.as_str(), i))
        .collect();

    // Build adjacency list: table_idx -> set of table indices it depends on
    let mut deps: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for table in tables {
        let idx = *table_map.get(table.name.as_str()).ok_or_else(|| {
            AppError::Internal(format!("Table '{}' not found in table_map", table.name))
        })?;
        deps.entry(idx).or_default();
        for fk in &table.foreign_keys {
            if let Some(dep_idx) = table_map.get(fk.references_table.as_str()) {
                deps.entry(idx).or_default().push(*dep_idx);
            }
        }
    }

    // Kahn's algorithm for topological sort.
    // Uses a Vec with an index pointer for O(1) dequeue, avoiding Vec::remove(0)'s O(n).
    let mut in_degree: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let mut reverse: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();

    for (idx, table_deps) in &deps {
        in_degree.entry(*idx).or_insert(0);
        for &dep in table_deps {
            reverse.entry(dep).or_default();
            let reverse_entry = reverse.get_mut(&dep).ok_or_else(|| {
                AppError::Internal("Missing reverse mapping during topological sort".to_string())
            })?;
            reverse_entry.push(*idx);
            *in_degree.entry(*idx).or_insert(0) += 1;
        }
    }

    // Collect initial zero-degree nodes, sorted for deterministic ordering.
    let mut queue: Vec<usize> = in_degree
        .iter()
        .filter(|&(_, &deg)| deg == 0)
        .map(|(&idx, _)| idx)
        .collect();
    queue.sort_unstable();

    let mut result = Vec::new();
    let mut head = 0usize;
    while head < queue.len() {
        // SAFETY: head < queue.len() is guaranteed by the while condition.
        let idx = queue[head];
        head += 1;
        result.push(idx);
        if let Some(dependents) = reverse.get(&idx) {
            for &dependent in dependents {
                let degree = in_degree.get_mut(&dependent).ok_or_else(|| {
                    AppError::Internal("Missing in_degree during topological sort".to_string())
                })?;
                *degree -= 1;
                if *degree == 0 {
                    // Insert in sorted position for deterministic ordering.
                    let new_val = dependent;
                    let insert_pos = queue[head..].partition_point(|&x| x < new_val);
                    queue.insert(head + insert_pos, new_val);
                }
            }
        }
    }

    // If there's a cycle, not all nodes were processed.
    if result.len() != tables.len() {
        let cycle_tables: Vec<&str> = tables
            .iter()
            .filter(|t| !result.contains(&table_map[t.name.as_str()]))
            .map(|t| t.name.as_str())
            .collect();
        return Err(AppError::Internal(format!(
            "Circular dependency detected among tables: [{}]",
            cycle_tables.join(", ")
        )));
    }

    // Build result in sorted order.
    // SAFETY: result contains indices into tables (from topological sort of table indices).
    Ok(result.iter().map(|&idx| tables[idx].clone()).collect())
}

// =============================================================================
// Media-specific column enforcement
// =============================================================================

/// Ensure media-specific columns exist on tables used by media endpoints.
///
/// The `file_path` column is required by the media handler for tracking
/// the stored file location, but it is not part of the standard media
/// presets (auto, tags, etc.). This function adds it if missing.
///
/// # Errors
///
/// Returns `AppError::Database` if the ALTER TABLE statement fails.
pub async fn ensure_media_columns<S: ::std::hash::BuildHasher + Sync>(
    endpoints: &[crate::config::types::EndpointConfig],
    pools: &HashMap<String, DatabasePool, S>,
) -> Result<(), AppError> {
    // Collect unique (database, table) pairs from media endpoints.
    let mut media_tables: Vec<(String, String)> = Vec::new();
    for endpoint in endpoints {
        if let Some(media) = &endpoint.media {
            let key = (media.database.clone(), media.table.clone());
            if !media_tables.contains(&key) {
                media_tables.push(key);
            }
        }
    }

    for (db_name, table_name) in &media_tables {
        let pool = pools.get(db_name).ok_or_else(|| {
            AppError::ConfigurationError(format!("Database '{db_name}' not found"))
        })?;

        let driver = pool.driver();
        let col_exists = match driver {
            DatabaseDriver::Sqlite => {
                let sql = format!(
                    "SELECT COUNT(*) as cnt FROM pragma_table_info({}) WHERE name = 'file_path'",
                    quote_identifier(table_name, driver)
                );
                let row = pool.fetch_optional_json(&sql, &[]).await?;
                row.and_then(|r| r.get("cnt").and_then(serde_json::Value::as_i64))
                    .is_some_and(|c| c > 0)
            }
            DatabaseDriver::Postgres => {
                let sql = "SELECT COUNT(*) as cnt FROM information_schema.columns \
                           WHERE table_name = $1 AND column_name = 'file_path'";
                let row = pool
                    .fetch_optional_json(sql, &[serde_json::Value::String(table_name.to_owned())])
                    .await?;
                row.and_then(|r| r.get("cnt").and_then(serde_json::Value::as_i64))
                    .is_some_and(|c| c > 0)
            }
            DatabaseDriver::Mysql => {
                let sql = "SELECT COUNT(*) as cnt FROM information_schema.columns \
                           WHERE table_name = ? AND column_name = 'file_path'";
                let row = pool
                    .fetch_optional_json(sql, &[serde_json::Value::String(table_name.to_owned())])
                    .await?;
                row.and_then(|r| r.get("cnt").and_then(serde_json::Value::as_i64))
                    .is_some_and(|c| c > 0)
            }
        };

        if !col_exists {
            let add_col_sql = format!(
                "ALTER TABLE {} ADD COLUMN \"file_path\" TEXT",
                quote_identifier(table_name, driver)
            );
            tracing::info!(
                "Adding 'file_path' column to media table '{table_name}' in database '{db_name}'"
            );
            tracing::debug!("DDL: {add_col_sql}");
            pool.execute_raw(&add_col_sql).await.map_err(|e| {
                AppError::ConfigurationError(format!(
                    "Failed to add file_path column to '{table_name}': {e}"
                ))
            })?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::ColumnConfig;

    #[test]
    fn test_generate_create_table_sqlite() {
        let table = TableConfig {
            name: "posts".to_string(),
            database: "main".to_string(),
            columns: vec![
                ColumnConfig {
                    name: "id".to_string(),
                    column_type: ColumnType::Integer,
                    primary_key: true,
                    nullable: false,
                    ..Default::default()
                },
                ColumnConfig {
                    name: "title".to_string(),
                    column_type: ColumnType::Text,
                    nullable: false,
                    ..Default::default()
                },
                ColumnConfig {
                    name: "count".to_string(),
                    column_type: ColumnType::Integer,
                    nullable: true,
                    default: Some("0".to_string()),
                    ..Default::default()
                },
            ],
            foreign_keys: vec![],
        };

        let sql = generate_create_table(&table, DatabaseDriver::Sqlite);
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS \"posts\""));
        assert!(sql.contains("\"id\" INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL"));
        assert!(sql.contains("\"title\" TEXT NOT NULL"));
        assert!(sql.contains("\"count\" INTEGER DEFAULT 0"));
        assert!(sql.contains("PRIMARY KEY"));
    }

    #[test]
    fn test_generate_create_table_postgres() {
        let table = TableConfig {
            name: "users".to_string(),
            database: "main".to_string(),
            columns: vec![
                ColumnConfig {
                    name: "id".to_string(),
                    column_type: ColumnType::Serial,
                    primary_key: true,
                    nullable: false,
                    ..Default::default()
                },
                ColumnConfig {
                    name: "email".to_string(),
                    column_type: ColumnType::Varchar,
                    unique: true,
                    nullable: false,
                    ..Default::default()
                },
            ],
            foreign_keys: vec![],
        };

        let sql = generate_create_table(&table, DatabaseDriver::Postgres);
        assert!(sql.contains("\"id\" SERIAL NOT NULL"));
        assert!(sql.contains("\"email\" VARCHAR(255) NOT NULL UNIQUE"));
    }

    #[test]
    fn test_column_type_mapping() {
        assert_eq!(
            column_type_to_sql(ColumnType::Timestamptz, DatabaseDriver::Postgres),
            "TIMESTAMPTZ"
        );
        assert_eq!(
            column_type_to_sql(ColumnType::Timestamptz, DatabaseDriver::Sqlite),
            "TEXT"
        );
        assert_eq!(
            column_type_to_sql(ColumnType::Boolean, DatabaseDriver::Sqlite),
            "INTEGER"
        );
        assert_eq!(
            column_type_to_sql(ColumnType::Jsonb, DatabaseDriver::Postgres),
            "JSONB"
        );
    }

    #[test]
    fn test_generate_add_column_nullable() {
        let col = ColumnConfig {
            name: "bio".to_string(),
            column_type: ColumnType::Text,
            nullable: true,
            ..Default::default()
        };
        let sql = generate_add_column("users", &col, DatabaseDriver::Sqlite);
        assert_eq!(sql, "ALTER TABLE \"users\" ADD COLUMN \"bio\" TEXT");
    }

    #[test]
    fn test_generate_add_column_not_null_with_default() {
        let col = ColumnConfig {
            name: "status".to_string(),
            column_type: ColumnType::Varchar,
            nullable: false,
            default: Some("'active'".to_string()),
            ..Default::default()
        };
        let sql = generate_add_column("users", &col, DatabaseDriver::Postgres);
        assert_eq!(
            sql,
            "ALTER TABLE \"users\" ADD COLUMN \"status\" VARCHAR(255) NOT NULL DEFAULT 'active'"
        );
    }

    #[test]
    fn test_generate_add_column_unique_postgres() {
        let col = ColumnConfig {
            name: "code".to_string(),
            column_type: ColumnType::Varchar,
            nullable: false,
            unique: true,
            default: Some("''".to_string()),
            ..Default::default()
        };
        let sql = generate_add_column("items", &col, DatabaseDriver::Postgres);
        assert!(sql.contains("UNIQUE"));
    }

    #[test]
    fn test_generate_add_column_unique_skipped_on_sqlite() {
        let col = ColumnConfig {
            name: "code".to_string(),
            column_type: ColumnType::Text,
            nullable: true,
            unique: true,
            ..Default::default()
        };
        let sql = generate_add_column("items", &col, DatabaseDriver::Sqlite);
        assert!(!sql.contains("UNIQUE"));
    }

    #[test]
    fn test_generate_drop_column() {
        let sql = generate_drop_column("users", "bio", DatabaseDriver::Sqlite);
        assert_eq!(sql, "ALTER TABLE \"users\" DROP COLUMN \"bio\"");
    }

    #[test]
    fn test_generate_drop_column_postgres() {
        let sql = generate_drop_column("users", "bio", DatabaseDriver::Postgres);
        assert_eq!(sql, "ALTER TABLE \"users\" DROP COLUMN \"bio\"");
    }

    #[test]
    fn test_generate_create_table_mysql_uses_backticks() {
        let table = TableConfig {
            name: "users".to_string(),
            database: "main".to_string(),
            columns: vec![ColumnConfig {
                name: "id".to_string(),
                column_type: ColumnType::Serial,
                primary_key: true,
                nullable: false,
                ..Default::default()
            }],
            foreign_keys: vec![],
        };

        let sql = generate_create_table(&table, DatabaseDriver::Mysql);
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS `users`"));
        assert!(sql.contains("`id` INTEGER AUTO_INCREMENT NOT NULL"));
    }

    #[test]
    fn test_generate_create_index_mysql_omits_if_not_exists() {
        let sql = generate_create_index("users", "email", DatabaseDriver::Mysql);
        assert_eq!(sql, "CREATE INDEX `idx_users_email` ON `users` (`email`)");
    }
}

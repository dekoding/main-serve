//! Database connection pool creation and lifecycle management.
//!
//! Wraps sqlx's driver-specific pools behind a unified `DatabasePool` enum.
//! Pools are created from YAML config and stored in `AppState` by name.
use std::collections::HashMap;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Column, Row};

use crate::config::types::{DatabaseConfig, DatabaseDriver};
use crate::error::AppError;

/// A database connection pool that abstracts over the supported backends.
#[derive(Debug, Clone)]
pub enum DatabasePool {
    /// `SQLite` connection pool.
    Sqlite(sqlx::SqlitePool),
    /// `PostgreSQL` connection pool.
    Postgres(sqlx::PgPool),
    /// `MySQL` connection pool.
    Mysql(sqlx::MySqlPool),
}

/// Dispatch a call across all `DatabasePool` variants.
macro_rules! dispatch {
    // Same body for every variant: dispatch!(self, |p| p.close().await)
    ($self:expr, |$p:ident| $body:expr) => {
        match $self {
            DatabasePool::Sqlite($p) => $body,
            DatabasePool::Postgres($p) => $body,
            DatabasePool::Mysql($p) => $body,
        }
    };
    // Per-variant free functions with shared args
    ($self:expr, $sq:expr, $pg:expr, $my:expr, $($arg:expr),+ $(,)?) => {
        match $self {
            DatabasePool::Sqlite(p) => $sq(p, $($arg),+).await,
            DatabasePool::Postgres(p) => $pg(p, $($arg),+).await,
            DatabasePool::Mysql(p) => $my(p, $($arg),+).await,
        }
    };
}

impl DatabasePool {
    /// Create a new pool from a database configuration entry.
    ///
    /// # Errors
    ///
    /// Returns `AppError::ConfigurationError` if the connection URL is invalid or the
    /// database cannot be reached.
    pub async fn connect(config: &DatabaseConfig) -> Result<Self, AppError> {
        match config.driver {
            DatabaseDriver::Sqlite => {
                let options: SqliteConnectOptions = config
                    .url
                    .parse::<SqliteConnectOptions>()
                    .map_err(|e| AppError::ConfigurationError(format!("Invalid SQLite URL: {e}")))?
                    .create_if_missing(true);
                let pool = SqlitePoolOptions::new()
                    .min_connections(config.min_connections)
                    .max_connections(config.max_connections)
                    .acquire_timeout(Duration::from_secs(config.acquire_timeout))
                    .connect_with(options)
                    .await
                    .map_err(|e| {
                        AppError::ConfigurationError(format!("Failed to connect to SQLite: {e}"))
                    })?;
                Ok(Self::Sqlite(pool))
            }
            DatabaseDriver::Postgres => {
                let pool = sqlx::postgres::PgPoolOptions::new()
                    .min_connections(config.min_connections)
                    .max_connections(config.max_connections)
                    .acquire_timeout(Duration::from_secs(config.acquire_timeout))
                    .connect(&config.url)
                    .await
                    .map_err(|e| {
                        AppError::ConfigurationError(format!("Failed to connect to Postgres: {e}"))
                    })?;
                Ok(Self::Postgres(pool))
            }
            DatabaseDriver::Mysql => {
                let pool = sqlx::mysql::MySqlPoolOptions::new()
                    .min_connections(config.min_connections)
                    .max_connections(config.max_connections)
                    .acquire_timeout(Duration::from_secs(config.acquire_timeout))
                    .connect(&config.url)
                    .await
                    .map_err(|e| {
                        AppError::ConfigurationError(format!("Failed to connect to MySQL: {e}"))
                    })?;
                Ok(Self::Mysql(pool))
            }
        }
    }

    /// Execute a raw SQL statement (used for migrations/DDL).
    ///
    /// # Errors
    ///
    /// Returns `AppError::Database` if the statement fails.
    pub async fn execute_raw(&self, sql: &str) -> Result<u64, AppError> {
        dispatch!(self, |p| {
            let result = sqlx::query(sql).execute(p).await?;
            Ok(result.rows_affected())
        })
    }

    /// Fetch rows from a raw SQL query, returning them as JSON values.
    ///
    /// # Errors
    ///
    /// Returns `AppError::Database` if the query fails.
    pub async fn fetch_all_json(
        &self,
        sql: &str,
        params: &[serde_json::Value],
    ) -> Result<Vec<serde_json::Value>, AppError> {
        dispatch!(
            self,
            fetch_sqlite_json,
            fetch_pg_json,
            fetch_mysql_json,
            sql,
            params
        )
    }

    /// Execute a SQL statement with parameters, returning rows affected.
    ///
    /// # Errors
    ///
    /// Returns `AppError::Database` if the statement fails.
    pub async fn execute_with_params(
        &self,
        sql: &str,
        params: &[serde_json::Value],
    ) -> Result<u64, AppError> {
        dispatch!(self, execute_sqlite, execute_pg, execute_mysql, sql, params)
    }

    /// Execute an INSERT statement and return the auto-generated last insert ID.
    ///
    /// For `MySQL`, retrieves the `last_insert_id` directly from the `QueryResult`
    /// to avoid connection-pool races with `SELECT LAST_INSERT_ID()`.
    /// For `SQLite`, runs `SELECT last_insert_rowid()` on the same connection.
    /// For Postgres, returns `0` (Postgres uses `RETURNING` instead).
    ///
    /// # Errors
    ///
    /// Returns `AppError::Database` if the statement fails.
    pub async fn execute_with_params_and_last_id(
        &self,
        sql: &str,
        params: &[serde_json::Value],
    ) -> Result<u64, AppError> {
        match self {
            Self::Sqlite(p) => execute_with_last_id_sqlite(p, sql, params).await,
            Self::Mysql(p) => execute_mysql_with_last_id(p, sql, params).await,
            Self::Postgres(_) => Ok(0),
        }
    }

    /// Fetch a single row as JSON, or None if not found.
    ///
    /// # Errors
    ///
    /// Returns `AppError::Database` if the query fails.
    pub async fn fetch_optional_json(
        &self,
        sql: &str,
        params: &[serde_json::Value],
    ) -> Result<Option<serde_json::Value>, AppError> {
        dispatch!(
            self,
            fetch_optional_sqlite_json,
            fetch_optional_pg_json,
            fetch_optional_mysql_json,
            sql,
            params
        )
    }

    /// Get the driver type of this pool.
    #[must_use]
    /// driver
    pub const fn driver(&self) -> DatabaseDriver {
        match self {
            Self::Sqlite(_) => DatabaseDriver::Sqlite,
            Self::Postgres(_) => DatabaseDriver::Postgres,
            Self::Mysql(_) => DatabaseDriver::Mysql,
        }
    }

    /// Gracefully close the pool, draining connections.
    ///
    /// # Errors
    ///
    /// Returns `AppError::Database` if the driver-specific pool close fails.
    pub async fn close(&self) -> Result<(), AppError> {
        dispatch!(self, |p| {
            p.close().await;
            Ok(())
        })
    }
}

/// Create all configured database pools, returning them by name.
///
/// # Errors
///
/// Returns `AppError::ConfigurationError` if any pool fails to connect.
pub async fn create_pools<S: std::hash::BuildHasher + Sync + Send + Default>(
    databases: &HashMap<String, DatabaseConfig, S>,
) -> Result<HashMap<String, DatabasePool, S>, AppError> {
    let mut pools: HashMap<_, _, S> = HashMap::default();
    for (name, config) in databases {
        tracing::info!("Connecting to database '{name}' ({:?})...", config.driver);
        let pool = DatabasePool::connect(config).await?;
        tracing::info!("Connected to database '{name}'");
        pools.insert(name.clone(), pool);
    }
    Ok(pools)
}

/// Gracefully close all pools.
pub async fn close_pools<S: std::hash::BuildHasher + Sync>(
    pools: &HashMap<String, DatabasePool, S>,
) {
    for (name, pool) in pools {
        tracing::info!("Closing database pool '{name}'...");
        if let Err(e) = pool.close().await {
            tracing::debug!("Failed to close pool '{name}': {e}");
        }
    }
}

// =============================================================================
// Driver-specific helpers (generated via macros to avoid triplication)
// =============================================================================

/// Generate `fetch_*_json`, `fetch_optional_*_json`, and `execute_*` functions
/// for a specific sqlx database driver.
macro_rules! impl_db_helpers {
    ($db:ty, $pool:ty, $args:ty, $row:ty,
     $fetch_fn:ident, $fetch_opt_fn:ident, $exec_fn:ident, $bind_fn:ident, $row_fn:ident) => {
        async fn $fetch_fn(
            pool: &$pool,
            sql: &str,
            params: &[serde_json::Value],
        ) -> Result<Vec<serde_json::Value>, AppError> {
            let mut query = sqlx::query(sql);
            for param in params {
                query = $bind_fn(query, param);
            }
            let rows = query.fetch_all(pool).await?;
            Ok(rows.iter().map($row_fn).collect())
        }

        async fn $fetch_opt_fn(
            pool: &$pool,
            sql: &str,
            params: &[serde_json::Value],
        ) -> Result<Option<serde_json::Value>, AppError> {
            let mut query = sqlx::query(sql);
            for param in params {
                query = $bind_fn(query, param);
            }
            let row = query.fetch_optional(pool).await?;
            Ok(row.as_ref().map($row_fn))
        }

        async fn $exec_fn(
            pool: &$pool,
            sql: &str,
            params: &[serde_json::Value],
        ) -> Result<u64, AppError> {
            let mut query = sqlx::query(sql);
            for param in params {
                query = $bind_fn(query, param);
            }
            let result = query.execute(pool).await?;
            Ok(result.rows_affected())
        }

        fn $bind_fn<'q>(
            query: sqlx::query::Query<'q, $db, $args>,
            param: &'q serde_json::Value,
        ) -> sqlx::query::Query<'q, $db, $args> {
            match param {
                serde_json::Value::Null => query.bind(None::<String>),
                serde_json::Value::Bool(b) => query.bind(*b),
                serde_json::Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        query.bind(i)
                    } else if let Some(f) = n.as_f64() {
                        query.bind(f)
                    } else {
                        query.bind(n.to_string())
                    }
                }
                serde_json::Value::String(s) => query.bind(s.as_str()),
                other => query.bind(other.to_string()),
            }
        }

        fn $row_fn(row: &$row) -> serde_json::Value {
            let columns = row.columns();
            let mut map = serde_json::Map::new();
            for col in columns {
                let name = col.name().to_string();
                // Use Option<T> variants so that SQL NULL is correctly
                // distinguished from empty/zero values on all backends
                // (SQLite in particular returns "" for NULL via try_get::<String>).
                // Try numeric types before String so that integer columns are
                // correctly represented as JSON numbers, even on backends where
                // try_get::<String> coerces numeric values (MySQL, Postgres).
                let value: serde_json::Value = row
                    .try_get::<Option<i64>, _>(col.ordinal())
                    .map(|opt| match opt {
                        Some(v) => serde_json::json!(v),
                        None => serde_json::Value::Null,
                    })
                    .or_else(|_| {
                        row.try_get::<Option<i32>, _>(col.ordinal())
                            .map(|opt| match opt {
                                Some(v) => serde_json::json!(v),
                                None => serde_json::Value::Null,
                            })
                    })
                    .or_else(|_| {
                        row.try_get::<Option<f64>, _>(col.ordinal())
                            .map(|opt| match opt {
                                Some(v) => serde_json::json!(v),
                                None => serde_json::Value::Null,
                            })
                    })
                    .or_else(|_| {
                        row.try_get::<Option<bool>, _>(col.ordinal())
                            .map(|opt| match opt {
                                Some(v) => serde_json::json!(v),
                                None => serde_json::Value::Null,
                            })
                    })
                    .or_else(|_| {
                        row.try_get::<Option<String>, _>(col.ordinal())
                            .map(|opt| match opt {
                                Some(s) => serde_json::Value::String(s),
                                None => serde_json::Value::Null,
                            })
                    })
                    .unwrap_or_else(|e| {
                        tracing::debug!(
                            "Row coercion failed for column at ordinal {}; \
                             falling back to NULL. Error: {e}",
                            col.ordinal()
                        );
                        serde_json::Value::Null
                    });
                map.insert(name, value);
            }
            serde_json::Value::Object(map)
        }
    };
}

impl_db_helpers!(
    sqlx::Sqlite,
    sqlx::SqlitePool,
    sqlx::sqlite::SqliteArguments<'q>,
    sqlx::sqlite::SqliteRow,
    fetch_sqlite_json,
    fetch_optional_sqlite_json,
    execute_sqlite,
    bind_sqlite_param,
    sqlite_row_to_json
);

impl_db_helpers!(
    sqlx::Postgres,
    sqlx::PgPool,
    sqlx::postgres::PgArguments,
    sqlx::postgres::PgRow,
    fetch_pg_json,
    fetch_optional_pg_json,
    execute_pg,
    bind_pg_param,
    pg_row_to_json
);

impl_db_helpers!(
    sqlx::MySql,
    sqlx::MySqlPool,
    sqlx::mysql::MySqlArguments,
    sqlx::mysql::MySqlRow,
    fetch_mysql_json,
    fetch_optional_mysql_json,
    execute_mysql,
    bind_mysql_param,
    mysql_row_to_json
);

/// Execute an INSERT and return the last insert ID directly from the
/// `QueryResult`. For `MySQL`, this avoids the connection-pool race where
/// `SELECT LAST_INSERT_ID()` could run on a different connection than the
/// INSERT itself.
async fn execute_mysql_with_last_id(
    pool: &sqlx::MySqlPool,
    sql: &str,
    params: &[serde_json::Value],
) -> Result<u64, AppError> {
    let mut query = sqlx::query(sql);
    for param in params {
        query = bind_mysql_param(query, param);
    }
    let result = query.execute(pool).await?;
    Ok(result.last_insert_id())
}

/// Execute an INSERT and return the last insert row ID.
/// For `SQLite`, this retrieves the `last_insert_rowid` directly from the
/// `QueryResult` on the same connection that performed the INSERT.
async fn execute_with_last_id_sqlite(
    pool: &sqlx::SqlitePool,
    sql: &str,
    params: &[serde_json::Value],
) -> Result<u64, AppError> {
    let mut query = sqlx::query(sql);
    for param in params {
        query = bind_sqlite_param(query, param);
    }
    let result = query.execute(pool).await?;
    Ok(result.last_insert_rowid().cast_unsigned())
}

/// Token revocation store query builders.
///
/// Contains functions for building SQL queries used by the database-backed
/// JWT token revocation store (`DatabaseRevocationStore` in `server/state.rs`).
/// Each function generates driver-specific SQL for the supported backends.
use crate::config::types::DatabaseDriver;
use crate::db::query::helpers::{now_expr, placeholder, quote_identifier};
use crate::db::query::types::BuiltQuery;
use crate::error::AppError;

/// Build `DELETE FROM {table} WHERE expires_at < {now}` for cleaning up expired revocations.
pub fn build_revoke_cleanup(table_name: &str, driver: DatabaseDriver) -> BuiltQuery {
    let now = now_expr(driver);
    BuiltQuery {
        sql: format!(
            "DELETE FROM {} WHERE expires_at < {}",
            quote_identifier(table_name, driver),
            now
        ),
        params: Vec::new(),
    }
}

/// Build `SELECT 1 FROM {table} WHERE jti = {param} LIMIT 1`.
///
/// Used to check if a token has been revoked.
pub fn build_revoke_check(
    table_name: &str,
    driver: DatabaseDriver,
    param_value: serde_json::Value,
) -> BuiltQuery {
    BuiltQuery {
        sql: format!(
            "SELECT 1 FROM {} WHERE jti = {} LIMIT 1",
            quote_identifier(table_name, driver),
            placeholder(driver, 1)
        ),
        params: vec![param_value],
    }
}

/// Build a driver-specific upsert for token revocation.
///
/// Generates the appropriate INSERT ... ON CONFLICT/REPLACE statement:
/// - SQLite: `INSERT OR REPLACE INTO ...`
/// - PostgreSQL: `INSERT INTO ... ON CONFLICT (jti) DO UPDATE SET expires_at = EXCLUDED.expires_at`
/// - MySQL: `INSERT INTO ... ON DUPLICATE KEY UPDATE expires_at = VALUES(expires_at)`
///
/// # Errors
///
/// Returns `AppError::BadRequest` if any of the string parameters are empty.
pub fn build_revoke_insert(
    table_name: &str,
    jti: &str,
    revoked_at: &str,
    expires_at: &str,
    driver: DatabaseDriver,
) -> Result<BuiltQuery, AppError> {
    if jti.is_empty() || revoked_at.is_empty() || expires_at.is_empty() {
        return Err(AppError::BadRequest(
            "JTI, revoked_at, and expires_at are required".to_string(),
        ));
    }

    let table = quote_identifier(table_name, driver);
    let (sql, _param_count) = match driver {
        DatabaseDriver::Sqlite => (
            format!(
                "INSERT OR REPLACE INTO {} (jti, revoked_at, expires_at) VALUES (?, ?, ?)",
                table
            ),
            3,
        ),
        DatabaseDriver::Postgres => (
            format!(
                "INSERT INTO {} (jti, revoked_at, expires_at) \
                 VALUES ($1, $2, $3) \
                 ON CONFLICT (jti) DO UPDATE SET expires_at = EXCLUDED.expires_at",
                table
            ),
            3,
        ),
        DatabaseDriver::Mysql => (
            format!(
                "INSERT INTO {} (jti, revoked_at, expires_at) \
                 VALUES (?, ?, ?) \
                 ON DUPLICATE KEY UPDATE expires_at = VALUES(expires_at)",
                table
            ),
            3,
        ),
    };

    // Parameters: jti, revoked_at, expires_at
    Ok(BuiltQuery {
        sql,
        params: vec![
            serde_json::Value::String(jti.to_owned()),
            serde_json::Value::String(revoked_at.to_owned()),
            serde_json::Value::String(expires_at.to_owned()),
        ],
    })
}

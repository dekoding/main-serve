/// Dedicated UPDATE query builders for common non-CRUD patterns.
///
/// These functions handle UPDATE queries that don't fit the general CRUD builder API,
/// such as soft-delete markers, trash operations, and single-column updates.
use crate::config::types::DatabaseDriver;
use crate::db::query::helpers::quote_identifier;
use crate::db::query::types::BuiltQuery;
use crate::error::AppError;

/// Generate the driver-appropriate NOW() equivalent expression.
///
/// SQLite uses `CURRENT_TIMESTAMP`, PostgreSQL and MySQL use `NOW()`.
fn now_expr(driver: DatabaseDriver) -> &'static str {
    match driver {
        DatabaseDriver::Sqlite => "CURRENT_TIMESTAMP",
        _ => "NOW()",
    }
}

/// Build `UPDATE {table} SET deleted_at = {now} WHERE id = {param}`.
///
/// Used by file_store.rs for soft-delete operations.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if `table_name` is empty.
pub fn build_set_deleted_at(
    table_name: &str,
    driver: DatabaseDriver,
) -> Result<BuiltQuery, AppError> {
    if table_name.is_empty() {
        return Err(AppError::BadRequest("Table name is required".to_string()));
    }
    let now = now_expr(driver);
    Ok(BuiltQuery {
        sql: format!(
            "UPDATE {} SET deleted_at = {} WHERE id = {}",
            quote_identifier(table_name, driver),
            now,
            placeholder_str(driver, 1)
        ),
        params: Vec::new(),
    })
}

/// Build `UPDATE {table} SET deleted_at = {now}, trashed_at = {now} WHERE id = {param}`.
///
/// Used by media/trash.rs when moving items to trash.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if `table_name` is empty.
pub fn build_set_trashed(
    table_name: &str,
    driver: DatabaseDriver,
) -> Result<BuiltQuery, AppError> {
    if table_name.is_empty() {
        return Err(AppError::BadRequest("Table name is required".to_string()));
    }
    let now = now_expr(driver);
    Ok(BuiltQuery {
        sql: format!(
            "UPDATE {} SET deleted_at = {}, trashed_at = {} WHERE id = {}",
            quote_identifier(table_name, driver),
            now,
            now,
            placeholder_str(driver, 1)
        ),
        params: Vec::new(),
    })
}

/// Build `UPDATE {table} SET trashed_at = NULL, deleted_at = NULL WHERE id = {param}`.
///
/// Used by media/trash.rs for restore operations.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if `table_name` is empty.
pub fn build_set_restored(
    table_name: &str,
    driver: DatabaseDriver,
) -> Result<BuiltQuery, AppError> {
    if table_name.is_empty() {
        return Err(AppError::BadRequest("Table name is required".to_string()));
    }
    Ok(BuiltQuery {
        sql: format!(
            "UPDATE {} SET trashed_at = NULL, deleted_at = NULL WHERE id = {}",
            quote_identifier(table_name, driver),
            placeholder_str(driver, 1)
        ),
        params: Vec::new(),
    })
}

/// Build `UPDATE {table} SET file_path = {new_path} WHERE id = {param}`.
///
/// Used by media/move_rename.rs for file path updates.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if `table_name` or `new_path` is empty.
pub fn build_set_file_path(
    table_name: &str,
    new_path: &str,
    driver: DatabaseDriver,
) -> Result<BuiltQuery, AppError> {
    if table_name.is_empty() || new_path.is_empty() {
        return Err(AppError::BadRequest(
            "table_name and new_path are required".to_string(),
        ));
    }

    Ok(BuiltQuery {
        sql: format!(
            "UPDATE {} SET file_path = {} WHERE id = {}",
            quote_identifier(table_name, driver),
            placeholder_str(driver, 1),
            placeholder_str(driver, 2)
        ),
        params: vec![serde_json::Value::String(new_path.to_owned())],
    })
}

/// Build `UPDATE {table} SET {column} = {value} WHERE id = {param}`.
///
/// Used by media/update.rs for single-column updates.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if `table_name` or `sets` is empty.
pub fn build_set_columns(
    table_name: &str,
    sets: &[(&str, serde_json::Value)],
    id: &str,
    driver: DatabaseDriver,
) -> Result<BuiltQuery, AppError> {
    if table_name.is_empty() || sets.is_empty() {
        return Err(AppError::BadRequest(
            "table_name and sets are required".to_string(),
        ));
    }

    let set_parts: Vec<String> = sets
        .iter()
        .enumerate()
        .map(|(i, (col, _))| {
            format!(
                "{} = {}",
                quote_identifier(col, driver),
                placeholder_str(driver, i + 1)
            )
        })
        .collect();

    let param_count = sets.len();
    let sql = format!(
        "UPDATE {} SET {} WHERE id = {}",
        quote_identifier(table_name, driver),
        set_parts.join(", "),
        placeholder_str(driver, param_count + 1)
    );

    let mut params: Vec<serde_json::Value> = sets.iter().map(|(_, val)| val.clone()).collect();
    params.push(serde_json::Value::String(id.to_owned()));

    Ok(BuiltQuery { sql, params })
}

/// Generate a placeholder string for a given parameter index.
fn placeholder_str(driver: DatabaseDriver, index: usize) -> String {
    match driver {
        DatabaseDriver::Postgres => format!("${index}"),
        DatabaseDriver::Sqlite | DatabaseDriver::Mysql => "?".to_string(),
    }
}

//! Update query builder.
//!
//! These functions handle UPDATE queries that don't fit the general CRUD builder API,
//! such as trash operations and file path updates.
use crate::config::types::DatabaseDriver;
use crate::db::query::helpers::{now_expr, placeholder, quote_identifier};
use crate::db::query::types::BuiltQuery;
use crate::error::AppError;

/// Build `UPDATE {table} SET deleted_at = {now}, trashed_at = {now} WHERE id = {param}`.
///
/// Used by media/trash.rs when moving items to trash.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if `table_name` is empty.
pub fn build_set_trashed(table_name: &str, driver: DatabaseDriver) -> Result<BuiltQuery, AppError> {
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
            placeholder(driver, 1)
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
            placeholder(driver, 1)
        ),
        params: Vec::new(),
    })
}

/// Build `UPDATE {table} SET file_path = {new_path} WHERE id = {param}`.
///
/// Used by `media/move_rename.rs` for file path updates.
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
            placeholder(driver, 1),
            placeholder(driver, 2)
        ),
        params: vec![serde_json::Value::String(new_path.to_owned())],
    })
}

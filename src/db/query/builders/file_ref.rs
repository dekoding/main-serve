//! File reference query builders for content references.
use crate::config::types::DatabaseDriver;
use crate::db::query::helpers::{placeholder, quote_identifier};
use crate::db::query::types::BuiltQuery;

/// Build `SELECT COALESCE(MAX({order_col}), 0) as max_order FROM {table}`.
///
/// Used to determine the next attachment order when adding file/media references.
#[must_use]
pub fn build_file_ref_max_order(
    table_name: &str,
    order_col: &str,
    driver: DatabaseDriver,
) -> BuiltQuery {
    let table = quote_identifier(table_name, driver);
    let order = quote_identifier(order_col, driver);
    BuiltQuery {
        sql: format!("SELECT COALESCE(MAX({order}), 0) as max_order FROM {table}"),
        params: Vec::new(),
    }
}

/// Build `SELECT {entity_id_col}, {content_type_col} FROM {table} WHERE {file_id_col} = ?`.
///
/// Used to retrieve all content references for a specific file/media ID.
#[must_use]
pub fn build_file_ref_select_by_file_id(
    table_name: &str,
    file_id_col: &str,
    entity_id_col: &str,
    content_type_col: &str,
    driver: DatabaseDriver,
) -> BuiltQuery {
    let table = quote_identifier(table_name, driver);
    let fid = quote_identifier(file_id_col, driver);
    let eid = quote_identifier(entity_id_col, driver);
    let ctc = quote_identifier(content_type_col, driver);
    BuiltQuery {
        sql: format!(
            "SELECT {eid}, {ctc} FROM {table} WHERE {fid} = {}",
            placeholder(driver, 1)
        ),
        params: Vec::new(),
    }
}

/// Build `INSERT INTO {table} ({columns}) VALUES ({placeholders})`.
///
/// Used for attaching file store entries to content entities.
/// Parameters: `file_id`, `entity_id`, `content_type`, order
#[must_use]
pub fn build_file_ref_insert(
    table_name: &str,
    columns: &[&str],
    driver: DatabaseDriver,
) -> BuiltQuery {
    let table = quote_identifier(table_name, driver);
    let col_names: Vec<String> = columns
        .iter()
        .map(|c| quote_identifier(c, driver))
        .collect();
    let placeholders: Vec<String> = (1..=columns.len())
        .map(|i| placeholder(driver, i))
        .collect();

    BuiltQuery {
        sql: format!(
            "INSERT INTO {} ({}) VALUES ({})",
            table,
            col_names.join(", "),
            placeholders.join(", ")
        ),
        params: Vec::new(),
    }
}

/// Used to retrieve a content reference after insertion.
///
/// Shared by media store and file store.
#[must_use]
pub fn build_file_ref_select(
    table_name: &str,
    file_id_col: &str,
    entity_id_col: &str,
    content_type_col: &str,
    driver: DatabaseDriver,
) -> BuiltQuery {
    let table = quote_identifier(table_name, driver);
    let fid = quote_identifier(file_id_col, driver);
    let eid = quote_identifier(entity_id_col, driver);
    let ctc = quote_identifier(content_type_col, driver);
    BuiltQuery {
        sql: format!(
            "SELECT * FROM {table} WHERE {fid} = {} AND {eid} = {} AND {ctc} = {}",
            placeholder(driver, 1),
            placeholder(driver, 2),
            placeholder(driver, 3)
        ),
        params: Vec::new(),
    }
}

/// Used for detaching file and media store entries from content entities.
#[must_use]
pub fn build_file_ref_delete(
    table_name: &str,
    file_id_col: &str,
    entity_id_col: &str,
    content_type_col: &str,
    driver: DatabaseDriver,
) -> BuiltQuery {
    let table = quote_identifier(table_name, driver);
    let fid = quote_identifier(file_id_col, driver);
    let eid = quote_identifier(entity_id_col, driver);
    let ctc = quote_identifier(content_type_col, driver);
    BuiltQuery {
        sql: format!(
            "DELETE FROM {table} WHERE {fid} = {} AND {eid} = {} AND {ctc} = {}",
            placeholder(driver, 1),
            placeholder(driver, 2),
            placeholder(driver, 3)
        ),
        params: Vec::new(),
    }
}

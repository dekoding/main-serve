/// Media entity reference query builders.
///
/// Handles INSERT, DELETE, and SELECT operations on the media-entity
/// association table (auto-created at startup during migration).
/// These queries use config-provided table and column names that are
/// validated at config load time.
use crate::config::types::DatabaseDriver;
use crate::db::query::helpers::{placeholder, quote_identifier};
use crate::db::query::types::BuiltQuery;

/// Build `SELECT COALESCE(MAX({order_col}), 0) as max_order FROM {table}`.
///
/// Used to determine the next attachment order when adding media references.
pub fn build_media_ref_max_order(
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

/// Build `INSERT INTO {table} ({columns}) VALUES ({placeholders})`.
///
/// Used for attaching media items to content entities.
/// Parameters: media_id, entity_id, content_type, order
pub fn build_media_ref_insert(
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

/// Build `SELECT * FROM {table} WHERE {media_id_col} = $1 AND {entity_id_col} = $2 AND {content_type_col} = $3`.
///
/// Used to retrieve a content reference after insertion.
pub fn build_media_ref_select(
    table_name: &str,
    media_id_col: &str,
    entity_id_col: &str,
    content_type_col: &str,
    driver: DatabaseDriver,
) -> BuiltQuery {
    let table = quote_identifier(table_name, driver);
    let mid = quote_identifier(media_id_col, driver);
    let eid = quote_identifier(entity_id_col, driver);
    let ctc = quote_identifier(content_type_col, driver);
    BuiltQuery {
        sql: format!(
            "SELECT * FROM {} WHERE {} = $1 AND {} = $2 AND {} = $3",
            table, mid, eid, ctc
        ),
        params: Vec::new(),
    }
}

/// Build `DELETE FROM {table} WHERE {media_id_col} = $1 AND {entity_id_col} = $2 AND {content_type_col} = $3`.
///
/// Used for detaching media items from content entities.
pub fn build_media_ref_delete(
    table_name: &str,
    media_id_col: &str,
    entity_id_col: &str,
    content_type_col: &str,
    driver: DatabaseDriver,
) -> BuiltQuery {
    let table = quote_identifier(table_name, driver);
    let mid = quote_identifier(media_id_col, driver);
    let eid = quote_identifier(entity_id_col, driver);
    let ctc = quote_identifier(content_type_col, driver);
    BuiltQuery {
        sql: format!(
            "DELETE FROM {} WHERE {} = $1 AND {} = $2 AND {} = $3",
            table, mid, eid, ctc
        ),
        params: Vec::new(),
    }
}

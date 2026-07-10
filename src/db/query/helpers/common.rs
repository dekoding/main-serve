use crate::config::types::{ColumnConfig, DatabaseDriver};

/// Check if a column in the table config is a JSON or JSONB type.
#[must_use]
pub(crate) fn is_jsonb_column(column_name: &str, columns: &[ColumnConfig]) -> bool {
    columns.iter().any(|c| {
        c.name == column_name
            && matches!(
                c.column_type,
                crate::config::types::ColumnType::Json | crate::config::types::ColumnType::Jsonb
            )
    })
}

/// Check if a filter column exists in the table schema.
#[must_use]
pub(crate) fn column_exists(column_name: &str, columns: &[ColumnConfig]) -> bool {
    columns.iter().any(|c| c.name == column_name)
}

/// Generate a driver-appropriate parameter placeholder.
#[must_use]
pub(crate) fn placeholder(driver: DatabaseDriver, index: usize) -> String {
    match driver {
        DatabaseDriver::Postgres => format!("${index}"),
        DatabaseDriver::Sqlite | DatabaseDriver::Mysql => "?".to_string(),
    }
}

/// Generate a driver-appropriate NOW() equivalent expression.
///
/// SQLite uses `CURRENT_TIMESTAMP`, PostgreSQL and MySQL use `NOW()`.
#[must_use]
pub(crate) fn now_expr(driver: DatabaseDriver) -> &'static str {
    match driver {
        DatabaseDriver::Sqlite => "CURRENT_TIMESTAMP",
        _ => "NOW()",
    }
}

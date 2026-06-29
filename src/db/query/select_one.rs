/// Dedicated SELECT query builders for common non-CRUD patterns.
///
/// These functions handle queries that don't fit the general CRUD builder API,
/// such as field-specific lookups, file path retrieval, and schema introspection.
use crate::config::types::DatabaseDriver;
use crate::db::query::helpers::{placeholder, quote_identifier};
use crate::db::query::types::BuiltQuery;
use crate::error::AppError;

/// Build `SELECT {columns} FROM {table} WHERE {field} = {param} LIMIT 1`.
///
/// Used by auth.rs for user lookups by email or other fields.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if `columns` or `field` is empty.
pub fn build_select_by_field(
    table_name: &str,
    columns: &[&str],
    field: &str,
    param_value: serde_json::Value,
    driver: DatabaseDriver,
) -> Result<BuiltQuery, AppError> {
    if columns.is_empty() {
        return Err(AppError::BadRequest("At least one column is required".to_string()));
    }
    if field.is_empty() {
        return Err(AppError::BadRequest("Field name is required".to_string()));
    }

    let cols = columns.iter().map(|c| quote_identifier(c, driver)).collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT {} FROM {} WHERE {} = {} LIMIT 1",
        cols,
        quote_identifier(table_name, driver),
        quote_identifier(field, driver),
        placeholder(driver, 1)
    );

    Ok(BuiltQuery {
        sql,
        params: vec![param_value],
    })
}

/// Build `SELECT file_path FROM {table} WHERE id = {param}`.
///
/// Used by media/delete.rs, media/resize.rs, media/move_rename.rs,
/// and file_store.rs to retrieve the stored file path.
pub fn build_select_file_path(
    table_name: &str,
    driver: DatabaseDriver,
) -> BuiltQuery {
    BuiltQuery {
        sql: format!(
            "SELECT file_path FROM {} WHERE id = {}",
            quote_identifier(table_name, driver),
            placeholder(driver, 1)
        ),
        params: Vec::new(),
    }
}

/// Build `SELECT id, file_path FROM {table} WHERE id = {param} AND trashed_at IS NOT NULL`.
///
/// Used by media/trash.rs to retrieve a trashed media item by ID.
pub fn build_select_trashed_item(
    table_name: &str,
    driver: DatabaseDriver,
) -> BuiltQuery {
    BuiltQuery {
        sql: format!(
            "SELECT id, file_path FROM {} WHERE id = {} AND trashed_at IS NOT NULL",
            quote_identifier(table_name, driver),
            placeholder(driver, 1)
        ),
        params: Vec::new(),
    }
}

/// Build `SELECT * FROM {table} WHERE trashed_at IS NOT NULL`.
///
/// Used by media/trash.rs to list all trashed media items with full metadata.
pub fn build_select_trashed(table_name: &str, driver: DatabaseDriver) -> BuiltQuery {
    BuiltQuery {
        sql: format!(
            "SELECT * FROM {} WHERE trashed_at IS NOT NULL",
            quote_identifier(table_name, driver)
        ),
        params: Vec::new(),
    }
}

/// Build `SELECT id FROM {table} WHERE trashed_at IS NOT NULL`.
///
/// Used by media/trash.rs to list all trashed item IDs for empty-trash operations.
pub fn build_select_trashed_ids(table_name: &str, driver: DatabaseDriver) -> BuiltQuery {
    BuiltQuery {
        sql: format!(
            "SELECT id FROM {} WHERE trashed_at IS NOT NULL",
            quote_identifier(table_name, driver)
        ),
        params: Vec::new(),
    }
}

/// Build `SELECT id, email, password_hash, role FROM {table} WHERE email = {param} LIMIT 1`.
///
/// Used by auth.rs for user login lookups.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if `table_name` is empty.
pub fn build_select_user_for_login(
    table_name: &str,
    driver: DatabaseDriver,
) -> Result<BuiltQuery, AppError> {
    if table_name.is_empty() {
        return Err(AppError::BadRequest("Table name is required".to_string()));
    }

    Ok(BuiltQuery {
        sql: format!(
            "SELECT id, email, password_hash, role FROM {} WHERE email = {} LIMIT 1",
            quote_identifier(table_name, driver),
            placeholder(driver, 1)
        ),
        params: Vec::new(),
    })
}

/// Build a driver-specific INSERT for user registration.
///
/// Inserts a new user with email, password_hash, and role. Optionally includes
/// created_at and updated_at timestamp columns with driver-appropriate
/// timestamp expressions (`CURRENT_TIMESTAMP` for SQLite, `NOW()` for PG/MySQL).
///
/// - SQLite: `INSERT INTO {table} (email, password_hash, role) VALUES (?, ?, ?)`
/// - PostgreSQL: `INSERT INTO {table} (email, password_hash, role) VALUES ($1, $2, $3)`
/// - MySQL: `INSERT INTO {table} (email, password_hash, role) VALUES (?, ?, ?)`
///
/// # Errors
///
/// Returns `AppError::BadRequest` if any string parameter is empty.
pub fn build_insert_user(
    table_name: &str,
    email: &str,
    password_hash: &str,
    role: &str,
    include_timestamps: bool,
    driver: DatabaseDriver,
) -> Result<BuiltQuery, AppError> {
    if table_name.is_empty() || email.is_empty() || password_hash.is_empty() || role.is_empty() {
        return Err(AppError::BadRequest(
            "table_name, email, password_hash, and role are required".to_string(),
        ));
    }

    let mut cols = vec![
        quote_identifier("email", driver),
        quote_identifier("password_hash", driver),
        quote_identifier("role", driver),
    ];
    let mut placeholders = vec![
        placeholder(driver, 1),
        placeholder(driver, 2),
        placeholder(driver, 3),
    ];

    if include_timestamps {
        let ts = match driver {
            DatabaseDriver::Sqlite => "CURRENT_TIMESTAMP".to_string(),
            _ => "NOW()".to_string(),
        };
        cols.push(quote_identifier("created_at", driver));
        cols.push(quote_identifier("updated_at", driver));
        placeholders.push(ts.clone());
        placeholders.push(ts);
    }

    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        quote_identifier(table_name, driver),
        cols.join(", "),
        placeholders.join(", "),
    );

    let params: Vec<serde_json::Value> = vec![
        email.to_string().into(),
        password_hash.to_string().into(),
        role.to_string().into(),
    ];

    Ok(BuiltQuery { sql, params })
}

/// Build `SELECT {columns} FROM {table} WHERE id = {param}`.
///
/// General-purpose single-field lookup by primary key.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if `columns` is empty.
pub fn build_select_by_id(
    table_name: &str,
    columns: &[&str],
    driver: DatabaseDriver,
) -> Result<BuiltQuery, AppError> {
    if columns.is_empty() {
        return Err(AppError::BadRequest("At least one column is required".to_string()));
    }

    let cols = columns.iter().map(|c| quote_identifier(c, driver)).collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT {} FROM {} WHERE id = {}",
        cols,
        quote_identifier(table_name, driver),
        placeholder(driver, 1)
    );

    Ok(BuiltQuery {
        sql,
        params: Vec::new(),
    })
}

/// Build schema introspection query for getting table columns.
///
/// Returns a list of column names via the `name` or `column_name` field.
/// Different SQL per driver (PRAGMA for SQLite, information_schema for PG/MySQL).
///
/// Returns `BuiltQuery` with no parameters (identifiers are validated at config time).
pub fn build_table_columns(table_name: &str, driver: DatabaseDriver) -> BuiltQuery {
    match driver {
        DatabaseDriver::Sqlite => BuiltQuery {
            sql: format!(
                "PRAGMA table_info({})",
                quote_identifier(table_name, driver)
            ),
            params: Vec::new(),
        },
        DatabaseDriver::Postgres => BuiltQuery {
            sql: "SELECT column_name FROM information_schema.columns \
                  WHERE table_name = $1 ORDER BY ordinal_position"
                .to_string(),
            params: vec![serde_json::Value::String(table_name.to_owned())],
        },
        DatabaseDriver::Mysql => BuiltQuery {
            sql: "SELECT column_name FROM information_schema.columns \
                  WHERE table_schema = DATABASE() AND table_name = ? \
                  ORDER BY ordinal_position"
                .to_string(),
            params: vec![serde_json::Value::String(table_name.to_owned())],
        },
    }
}

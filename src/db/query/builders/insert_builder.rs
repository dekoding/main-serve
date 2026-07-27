use crate::config::types::{ColumnType, DatabaseDriver};
use crate::db::query::helpers::{
    coerce_filter_value_by_type, find_pk_column, interpolate_value, is_valid_identifier,
    placeholder, quote_identifier, resolve_writable_fields,
};
use crate::db::query::types::{BuiltQuery, MutationContext};
use crate::error::AppError;
use crate::handlers::common::utils::DatabaseContext;
use crate::middleware::auth::extractor::RequestContext;

/// Build an INSERT query from a JSON body.
///
/// Handles auto-population of the owner field via `insert_owner`,
/// validates field names, handles JSONB column casting for `PostgreSQL`,
/// and generates the appropriate `RETURNING` clause per driver.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if the body is not a JSON object, contains
/// invalid field names, or provides no writable fields.
/// Returns `AppError::Internal` if the table has no primary key column.
pub fn build_insert(
    db_ctx: &DatabaseContext,
    ctx: &MutationContext,
    body: &serde_json::Value,
    context: &RequestContext,
) -> Result<BuiltQuery, AppError> {
    let table_config = &db_ctx.table_config;
    let driver = db_ctx.pool.driver();
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::BadRequest("Request body must be a JSON object".to_string()))?;

    let writable = resolve_writable_fields(&ctx.writable_fields, &db_ctx.table_config);

    // Check if we need to auto-populate the owner field.
    let owner_col = ctx.insert_owner.clone();

    let mut columns: Vec<String> = Vec::new();
    let mut placeholders: Vec<String> = Vec::new();
    let mut params: Vec<serde_json::Value> = Vec::new();
    let mut param_idx = 1usize;

    // Auto-populate owner field if configured. Server always controls ownership.
    if let Some(ref owner_field) = owner_col
        && let Some(user_id) = &context.user_id
    {
        let owner_col_type = table_config
            .columns
            .iter()
            .find(|c| c.name == *owner_field)
            .map(|c| &c.column_type);

        let coerced = owner_col_type.map_or_else(
            || serde_json::Value::String(user_id.clone()),
            |ct| coerce_filter_value_by_type(user_id, *ct, db_ctx.pool.driver()),
        );
        columns.push(owner_field.clone());
        let placeholder = placeholder(driver, param_idx);
        placeholders.push(placeholder);
        params.push(coerced);
        param_idx += 1;
    }

    for (key, value) in obj {
        // Skip the owner field - it is auto-populated by insert_owner.
        if let Some(ref owner_field) = owner_col
            && key == owner_field
        {
            continue;
        }

        if !writable.contains(key) {
            continue;
        }
        if !is_valid_identifier(key) {
            return Err(AppError::BadRequest(format!("Invalid field name: {key}")));
        }
        columns.push(key.clone());

        // Handle interpolation for string values in the request body.
        let final_value = if let Some(s) = value.as_str() {
            interpolate_value(s, context)?
        } else {
            value.clone()
        };

        let placeholder = if driver == DatabaseDriver::Postgres {
            // Check if this column is a JSONB type
            let is_jsonb = table_config
                .columns
                .iter()
                .any(|c| c.name == *key && matches!(c.column_type, ColumnType::Jsonb));
            if is_jsonb {
                format!("{}::jsonb", placeholder(driver, param_idx))
            } else {
                placeholder(driver, param_idx)
            }
        } else {
            placeholder(driver, param_idx)
        };

        placeholders.push(placeholder);
        params.push(final_value);
        param_idx += 1;
    }

    if columns.is_empty() {
        return Err(AppError::BadRequest(
            "No writable fields provided in request body".to_string(),
        ));
    }

    let pk_col = find_pk_column(table_config)?;
    let returning = match driver {
        DatabaseDriver::Postgres | DatabaseDriver::Sqlite => owner_col.map_or_else(
            || format!(" RETURNING {}", quote_identifier(&pk_col, driver)),
            |of| {
                format!(
                    " RETURNING {}, {}",
                    quote_identifier(&pk_col, driver),
                    quote_identifier(&of, driver)
                )
            },
        ),
        DatabaseDriver::Mysql => String::new(),
    };

    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({}){}",
        table_config.name,
        columns.join(", "),
        placeholders.join(", "),
        returning
    );

    Ok(BuiltQuery { sql, params })
}

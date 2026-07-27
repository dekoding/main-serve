use crate::config::types::{ColumnType, DatabaseDriver};
use crate::db::query::helpers::{
    coerce_pk_value, find_pk_column, interpolate_value, interpolate_where_clause,
    is_valid_identifier, placeholder, quote_identifier, resolve_writable_fields,
};
use crate::db::query::types::{BuiltQuery, MutationContext};
use crate::error::AppError;
use crate::handlers::common::utils::DatabaseContext;
use crate::middleware::auth::extractor::RequestContext;

/// Build an UPDATE query from a JSON body, targeting a single record by PK.
///
/// Handles JSONB column casting for PostgreSQL and uses a sentinel WHERE clause
/// (`1 = 0`) when PK coercion fails to prevent type mismatch errors.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if the body is not a JSON object, contains
/// invalid field names, or provides no writable fields.
/// Returns `AppError::Internal` if the table has no primary key column.
pub fn build_update(
    db_ctx: &DatabaseContext,
    ctx: MutationContext,
    pk_value: &str,
    body: &serde_json::Value,
    context: &RequestContext,
    where_clause: &Option<String>,
) -> Result<BuiltQuery, AppError> {
    let table_name = &db_ctx.table_config.name;
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::BadRequest("Request body must be a JSON object".to_string()))?;

    let writable = resolve_writable_fields(&ctx.writable_fields, &db_ctx.table_config);
    let pk_col = find_pk_column(&db_ctx.table_config)?;
    let mut set_parts: Vec<String> = Vec::new();
    let mut params: Vec<serde_json::Value> = Vec::new();
    let mut param_idx = 1usize;

    for (key, value) in obj {
        if !writable.contains(key) {
            continue;
        }
        if !is_valid_identifier(key) {
            return Err(AppError::BadRequest(format!("Invalid field name: {key}")));
        }

        // Handle interpolation for string values in the request body.
        let final_value = if let Some(s) = value.as_str() {
            interpolate_value(s, context)?
        } else {
            value.clone()
        };

        // Postgres requires an explicit cast to JSONB when binding a text
        // parameter to a JSONB column.
        let set_value = if db_ctx.pool.driver() == DatabaseDriver::Postgres
            && db_ctx
                .table_config
                .columns
                .iter()
                .any(|c| c.name == *key && matches!(c.column_type, ColumnType::Jsonb))
        {
            format!("{}::jsonb", placeholder(db_ctx.pool.driver(), param_idx))
        } else {
            placeholder(db_ctx.pool.driver(), param_idx)
        };

        set_parts.push(format!("{} = {}", key, set_value));
        params.push(final_value);
        param_idx += 1;
    }

    if set_parts.is_empty() {
        return Err(AppError::BadRequest(
            "No writable fields provided in request body".to_string(),
        ));
    }

    let pk_val = coerce_pk_value(&db_ctx.table_config, pk_value);
    let is_coercion_sentinel = pk_val
        .as_number()
        .is_some_and(|n| n.as_i64() == Some(i64::MIN));
    // "1 = 0" is a sentinel WHERE clause that matches zero rows.
    // This is used when PK coercion fails (e.g. type mismatch) to prevent
    // the UPDATE from affecting any row instead of failing silently.
    let mut sql = if is_coercion_sentinel {
        format!(
            "UPDATE {} SET {} WHERE 1 = 0",
            quote_identifier(table_name, db_ctx.pool.driver()),
            set_parts.join(", ")
        )
    } else {
        format!(
            "UPDATE {} SET {} WHERE {} = {}",
            table_name,
            set_parts.join(", "),
            pk_col,
            placeholder(db_ctx.pool.driver(), param_idx)
        )
    };
    if !is_coercion_sentinel {
        params.push(pk_val);
    }

    if let Some(wc) = where_clause {
        let mut wc_sql_parts: Vec<String> = Vec::new();
        interpolate_where_clause(
            wc,
            context,
            db_ctx.pool.driver(),
            &mut params,
            &mut wc_sql_parts,
        )?;
        sql = format!("{sql} AND {}", wc_sql_parts.join(""));
    }

    Ok(BuiltQuery { sql, params })
}

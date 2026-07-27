use crate::db::query::helpers::{
    coerce_pk_value, find_pk_column, interpolate_where_clause, placeholder, quote_identifier,
};
use crate::db::query::types::BuiltQuery;
use crate::error::AppError;
use crate::handlers::common::utils::DatabaseContext;
use crate::middleware::auth::extractor::RequestContext;

/// Build a DELETE query targeting a single record by PK.
///
/// Uses a sentinel WHERE clause (`1 = 0`) when PK coercion fails to prevent
/// type mismatch errors on PostgreSQL when binding a non-numeric string
/// to an integer column.
///
/// # Errors
///
/// Returns `AppError::Internal` if the table has no primary key column.
pub fn build_delete(
    pk_value: &str,
    db_ctx: &DatabaseContext,
    context: &RequestContext,
    where_clause: &Option<String>,
) -> Result<BuiltQuery, AppError> {
    let table_name = &db_ctx.table_config.name;
    let pk_col = find_pk_column(&db_ctx.table_config)?;
    let pk_val = coerce_pk_value(&db_ctx.table_config, pk_value);
    let is_coercion_sentinel = pk_val
        .as_number()
        .is_some_and(|n| n.as_i64() == Some(i64::MIN));

    if is_coercion_sentinel {
        let mut delete_params: Vec<serde_json::Value> = Vec::new();
        let sql = if let Some(wc) = where_clause {
            let mut wc_sql_parts: Vec<String> = Vec::new();
            interpolate_where_clause(
                wc,
                context,
                db_ctx.pool.driver(),
                &mut delete_params,
                &mut wc_sql_parts,
            )?;
            format!(
                "DELETE FROM {} WHERE 1 = 0 AND {}",
                quote_identifier(table_name, db_ctx.pool.driver()),
                wc_sql_parts.join("")
            )
        } else {
            format!(
                "DELETE FROM {} WHERE 1 = 0",
                quote_identifier(table_name, db_ctx.pool.driver())
            )
        };
        return Ok(BuiltQuery {
            sql,
            params: delete_params,
        });
    }

    let mut params: Vec<serde_json::Value> = vec![pk_val];
    let mut base_sql = format!(
        "DELETE FROM {} WHERE {} = {}",
        table_name,
        pk_col,
        placeholder(db_ctx.pool.driver(), 1)
    );

    if let Some(wc) = where_clause {
        let mut wc_sql_parts: Vec<String> = Vec::new();
        interpolate_where_clause(
            wc,
            context,
            db_ctx.pool.driver(),
            &mut params,
            &mut wc_sql_parts,
        )?;
        base_sql = format!("{base_sql} AND {}", wc_sql_parts.join(""));
    }

    Ok(BuiltQuery {
        sql: base_sql,
        params,
    })
}

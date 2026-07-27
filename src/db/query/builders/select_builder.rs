use crate::db::query::helpers::{coerce_pk_value, find_pk_column};
use crate::db::query::select::SelectBuilder;
use crate::db::query::types::{BuiltQuery, SelectContext};
use crate::error::AppError;
use crate::handlers::common::utils::DatabaseContext;
use crate::middleware::auth::extractor::RequestContext;

/// Build a SELECT query for listing records.
///
/// Delegates to `SelectBuilder` which accumulates fields, joins, computed
/// fields, WHERE conditions, filters, sorting, and pagination.
///
/// # Errors
///
/// Returns `AppError::BadRequest` if filter or sort fields are invalid or disallowed.
pub fn build_select_list(
    db_ctx: &DatabaseContext,
    ctx: &SelectContext,
    query_params: &crate::db::query::types::QueryParams,
    context: &RequestContext,
) -> Result<BuiltQuery, AppError> {
    let table_name = &db_ctx.table_config.name;
    use crate::db::query::helpers::resolve_fields;

    let fields = resolve_fields(&ctx.fields, &db_ctx.table_config);
    let mut sb = SelectBuilder::new(table_name, fields, db_ctx.pool.driver());

    sb.apply_joins(ctx);
    sb.apply_computed_fields(ctx);
    sb.apply_where_clause(ctx, context)?;
    sb.apply_filters(ctx, &query_params.filters, &db_ctx.table_config)?;
    sb.apply_sorting(ctx, &db_ctx.table_config, query_params)?;
    sb.apply_pagination(ctx, query_params);

    Ok(sb.build())
}

/// Build a SELECT query for a single record by primary key.
///
/// Uses `SelectBuilder` with a single PK condition and a hard LIMIT 1.
///
/// # Errors
///
/// Returns `AppError::Internal` if the table has no primary key column.
pub fn build_select_one(
    db_ctx: &DatabaseContext,
    ctx: &SelectContext,
    pk_value: &str,
    context: &RequestContext,
) -> Result<BuiltQuery, AppError> {
    let table_config = &db_ctx.table_config;
    let table_name = &table_config.name;
    use crate::db::query::helpers::resolve_fields;

    let fields = resolve_fields(&ctx.fields, table_config);
    let pk_col = find_pk_column(table_config)?;
    let mut sb = SelectBuilder::new(table_name, fields, db_ctx.pool.driver());

    sb.apply_pk_condition(&pk_col, coerce_pk_value(table_config, pk_value));
    sb.apply_where_clause(ctx, context)?;
    sb.limit_one();

    Ok(sb.build())
}

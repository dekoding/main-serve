/// COUNT query builder for list pagination.
///
/// Generates `SELECT COUNT(*) ...` with the same WHERE clause filters as the
/// corresponding list query, ensuring JSONB support, bracket notation, and
/// multiple filter handling are consistent with the full `SelectBuilder` pipeline.
use crate::config::types::{DatabaseDriver, TableConfig};
use crate::db::query::select::SelectBuilder;
use crate::db::query::types::{BuiltQuery, QueryParams, SelectContext};
use crate::middleware::auth::extractor::RequestContext;

/// Build a COUNT query for list pagination.
///
/// Reuses `SelectBuilder` to apply the config `where_clause` and all filter
/// conditions through the same pipeline as the list query, guaranteeing that
/// JSONB nested paths, LHS bracket notation, and multiple filters produce
/// identical SQL to `build_select_list`.
pub fn build_select_list_count(
    table_name: &str,
    driver: DatabaseDriver,
    ctx: &SelectContext,
    query_params: &QueryParams,
    context: &RequestContext,
    table_config: &TableConfig,
) -> Result<BuiltQuery, crate::error::AppError> {
    let fields = vec!["COUNT(*) as count".to_string()];
    let mut sb = SelectBuilder::new(table_name, fields, driver);
    sb.apply_where_clause(ctx, context)?;
    sb.apply_filters(ctx, &query_params.filters, table_config)?;
    Ok(sb.build())
}

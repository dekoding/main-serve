use crate::config::types::DatabaseDriver;
use crate::db::query::helpers::placeholder;
use crate::db::query::types::{QueryParams, SelectContext};
use crate::error::AppError;
use crate::middleware::auth::extractor::RequestContext;

use super::validation::quote_identifier;

/// Build a COUNT query for list pagination.
///
/// This includes the same WHERE clause filters as the original list query.
pub fn build_select_list_count(
    table_name: &str,
    driver: DatabaseDriver,
    query_params: &QueryParams,
    ctx: &SelectContext,
    context: &RequestContext,
) -> Result<crate::db::query::types::BuiltQuery, AppError> {
    use crate::db::query::select::SelectBuilder;

    let mut sb = SelectBuilder::new(table_name, vec!["COUNT(*) as count".to_string()], driver);
    sb.apply_where_clause(ctx, context)?;

    // Apply query param filters without table validation.
    for (key, value) in &query_params.filters {
        if ["page", "page_size", "per_page", "sort", "order"].contains(&key.as_str()) {
            continue;
        }
        // Simple equality filter for count query
        let ph = placeholder(driver, 1);
        let sql = format!(
            "SELECT COUNT(*) as count FROM {} WHERE {} = {}",
            quote_identifier(table_name, driver),
            quote_identifier(key, driver),
            ph
        );
        return Ok(crate::db::query::types::BuiltQuery {
            sql,
            params: vec![serde_json::Value::String(value.clone())],
        });
    }

    // If no filters, just count everything.
    let sql = format!(
        "SELECT COUNT(*) as count FROM {}",
        quote_identifier(table_name, driver)
    );
    Ok(crate::db::query::types::BuiltQuery {
        sql,
        params: vec![],
    })
}

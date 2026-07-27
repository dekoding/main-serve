use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::db::query::builders::build_select_list;
use crate::db::query::helpers::extract_query_params;
use crate::db::query::types::SelectContext;
use crate::error::AppError;
use crate::handlers::file_store::{FileStoreContext, apply_row_permissions};
use crate::middleware::auth::extractor::RequestContext;

/// Handle listing file store entries.
pub async fn handle_file_store_list(ctx: &FileStoreContext<'_>) -> Result<Response, AppError> {
    let pool = &ctx.db_ctx.pool;
    let mut qp = extract_query_params(ctx.handler_ctx.query_params);

    qp.page.get_or_insert(1);
    qp.page_size
        .get_or_insert(ctx.config.pagination.default_page_size);

    if qp.sort.is_none() && !ctx.config.sorting.default_field.is_empty() {
        qp.sort = Some(ctx.config.sorting.default_field.clone());
    }
    if qp.order.is_none() {
        qp.order = Some(ctx.config.sorting.default_order);
    }

    if let Some(ownership) = &ctx.config.ownership {
        let user_id = ctx.handler_ctx.extract_user_id().await?;
        let auth_info = ctx.handler_ctx.extract_auth_info().await?;

        if !ctx.handler_ctx.endpoint.roles.is_admin(&auth_info.role) {
            let owner_col = ownership.owner_column.as_str();
            qp.filters.insert(owner_col.to_string(), user_id);
        }
    }

    let select_ctx = SelectContext::permissive();
    let built = build_select_list(ctx.db_ctx, &select_ctx, &qp, &RequestContext::default())?;
    let mut rows = pool.fetch_all_json(&built.sql, &built.params).await?;

    // Filter out trashed items
    rows.retain(|row| row.get("trashed_at").and_then(|v| v.as_str()).is_none());

    let total: i64 = rows.len() as i64;

    // Convert numeric ids to strings for consistency with create/get responses
    for row in &mut rows {
        if let serde_json::Value::Object(obj) = row
            && let Some(id_val) = obj.get("id")
        {
            let id_str = id_val
                .as_i64()
                .map(|i| i.to_string())
                .unwrap_or_else(|| id_val.to_string());
            obj.insert("id".to_string(), serde_json::Value::String(id_str));
        }
    }

    let rows = apply_row_permissions(&rows, ctx.config).await?;

    let page = qp.page.unwrap_or(ctx.config.pagination.default_page_size);
    let page_size = qp
        .page_size
        .unwrap_or(ctx.config.pagination.default_page_size);

    let response = serde_json::json!({
        "data": rows,
        "pagination": {
            "page": page,
            "page_size": page_size,
            "total": total,
            "total_pages": (total as f64 / page_size as f64).ceil() as u64,
        }
    });

    tracing::debug!(
        "LIST rows count={}, first_row={:?}",
        rows.len(),
        rows.first()
    );

    Ok((StatusCode::OK, axum::Json(response)).into_response())
}

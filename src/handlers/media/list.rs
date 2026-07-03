/// Media list and get handlers.
use std::collections::HashMap;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::MediaConfig;
use crate::db::query::builders::{build_select_list, build_select_one};
use crate::db::query::helpers::build_select_list_count;
use crate::db::query::types::{QueryParams, SelectContext};
use crate::error::AppError;
use crate::handlers::common::utils::DatabaseContext;
use crate::middleware::auth::extractor::RequestContext;

/// Handle media list (GET /).
pub async fn handle_media_list(
    db_context: &DatabaseContext,
    config: &MediaConfig,
    query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    let page = query_params
        .get("page")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1);

    let page_size = query_params
        .get("page_size")
        .or_else(|| query_params.get("per_page"))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(config.pagination.default_page_size);

    let sort = query_params.get("sort").cloned().or_else(|| {
        if !config.sorting.default_field.is_empty() {
            Some(config.sorting.default_field.clone())
        } else {
            None
        }
    });

    let order = Some(config.sorting.default_order);

    let reserved = ["page", "page_size", "per_page", "sort", "order"];
    let filters: HashMap<String, String> = query_params
        .iter()
        .filter(|(k, _)| !reserved.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let qp = QueryParams {
        page: Some(page),
        page_size: Some(page_size),
        sort,
        order,
        filters,
    };

    let built = build_select_list(
        db_context,
        &SelectContext::permissive(),
        &qp,
        &RequestContext::default(),
    )?;
    let rows = db_context
        .pool
        .fetch_all_json(&built.sql, &built.params)
        .await?;

    let count_built = build_select_list_count(
        &config.table,
        db_context.pool.driver(),
        &qp,
        &SelectContext::permissive(),
        &RequestContext::default(),
    )?;
    let count_row = db_context
        .pool
        .fetch_optional_json(&count_built.sql, &count_built.params)
        .await?;
    let total: i64 = count_row
        .as_ref()
        .and_then(|r| r.get("count"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let response = serde_json::json!({
        "data": rows,
        "pagination": {
            "page": page,
            "page_size": page_size,
            "total": total,
            "total_pages": (total as f64 / page_size as f64).ceil() as u64,
        }
    });

    Ok((StatusCode::OK, axum::Json(response)).into_response())
}

/// Handle media get by ID.
pub async fn handle_media_get(
    db_context: &DatabaseContext,
    id: &str,
) -> Result<Response, AppError> {
    let built = build_select_one(
        db_context,
        &SelectContext::permissive(),
        id,
        &RequestContext::default(),
    )?;
    match db_context
        .pool
        .fetch_optional_json(&built.sql, &built.params)
        .await?
    {
        Some(row) => Ok((StatusCode::OK, axum::Json(row)).into_response()),
        None => Err(AppError::NotFound(format!(
            "Media item with id '{}' not found",
            &id
        ))),
    }
}

/// Generic CRUD handlers driven by YAML endpoint configuration.
///
/// Each CRUD endpoint is dispatched by HTTP method:
/// - GET (no path param)  -> list records (with pagination, filtering, sorting)
/// - GET (with path param) -> get single record by PK
/// - POST                  -> insert new record
/// - PUT / PATCH           -> update record by PK
/// - DELETE                -> delete record by PK
use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::EndpointConfig;
use crate::db::query::builders::{
    build_delete, build_insert, build_select_list, build_select_one, build_update,
};
use crate::db::query::helpers::{build_select_list_count, extract_query_params};
use crate::db::query::types::{MutationContext, SelectContext};
use crate::error::AppError;
use crate::handlers::common::utils::{DatabaseContext, get_db_context};
use crate::middleware::auth::extractor::RequestContext;
use crate::server::state::AppState;

/// Handle a CRUD endpoint - dispatches by method and path params.
///
/// # Errors
///
/// Returns `AppError::Internal` if the table or database pool is missing.
/// Returns `AppError::BadRequest` for invalid request bodies or unsupported methods.
/// Returns `AppError::NotFound` if a targeted record does not exist.
pub async fn handle_crud(
    State(state): State<AppState>,
    method: axum::http::Method,
    path_params: Option<Path<HashMap<String, String>>>,
    Query(query_string): Query<HashMap<String, String>>,
    body: Option<Json<serde_json::Value>>,
    endpoint: EndpointConfig,
    context: RequestContext,
) -> Result<Response, AppError> {
    let crud = endpoint
        .crud
        .as_ref()
        .ok_or_else(|| AppError::Internal("CRUD config missing on crud endpoint".to_string()))?;

    let select_ctx = SelectContext::from(crud);
    let mutate_ctx = MutationContext::from(crud);

    let db_context = get_db_context(&state, crud.database.clone(), crud.table.clone()).await?;

    let pk_value = path_params.as_ref().and_then(|p| p.get("id").cloned());

    match (method.as_str(), pk_value.as_deref()) {
        ("GET", None) => handle_list(&db_context, &select_ctx, crud, &query_string, &context).await,
        ("GET", Some(_)) => {
            handle_get_one(&db_context, &select_ctx, crud, &pk_value, &context).await
        }
        ("POST", _) => handle_create(&db_context, &mutate_ctx, &body, &context).await,
        ("PUT" | "PATCH", Some(_)) => {
            handle_update(&db_context, mutate_ctx, crud, &pk_value, &body, &context).await
        }
        ("DELETE", Some(_)) => handle_delete(&db_context, crud, &pk_value, &context).await,
        _ => Err(AppError::MethodNotAllowed(
            "Unsupported method for CRUD endpoint".to_string(),
        )),
    }
}

/// Handle list records (GET without path param).
///
/// # Errors
///
/// Returns `AppError::Internal` for database or query-building failures.
async fn handle_list(
    db_context: &DatabaseContext,
    select_ctx: &SelectContext,
    crud: &crate::config::types::CrudConfig,
    query_string: &HashMap<String, String>,
    context: &RequestContext,
) -> Result<Response, AppError> {
    let qp = extract_query_params(query_string);
    let built = match build_select_list(db_context, select_ctx, &qp, context) {
        Ok(q) => q,
        Err(e) => {
            tracing::error!("build_select_list failed: {:?}", e);
            return Err(e);
        }
    };

    let rows = match db_context
        .pool
        .fetch_all_json(&built.sql, &built.params)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(
                "Database error - SQL: {}, params: {:?}, error: {}",
                built.sql,
                built.params,
                e
            );
            return Err(e);
        }
    };

    let pagination = &crud.pagination;
    let default_page_size = pagination.default_page_size;
    let max_page_size = pagination.max_page_size;
    let effective_page_size = qp
        .page_size
        .unwrap_or(default_page_size)
        .min(max_page_size)
        .max(1);
    let page = qp.page.unwrap_or(1);

    let count_q = build_select_list_count(
        &db_context.table_config.name,
        db_context.pool.driver(),
        &qp,
        select_ctx,
        context,
    );
    let total = match count_q {
        Ok(count_build) => match db_context
            .pool
            .fetch_optional_json(&count_build.sql, &count_build.params)
            .await
        {
            Ok(Some(row)) => row.get("count").and_then(|v| v.as_u64()).unwrap_or(0),
            _ => 0,
        },
        Err(e) => {
            tracing::debug!("Failed to build count query: {e}");
            0
        }
    };

    let response = if pagination.enabled {
        let total_pages = if effective_page_size > 0 {
            (total as u64).div_ceil(effective_page_size)
        } else {
            1
        };
        serde_json::json!({
            "data": rows,
            "total": total,
            "page": page,
            "page_size": effective_page_size,
            "total_pages": total_pages,
        })
    } else {
        serde_json::json!({ "data": rows })
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle get single record by PK (GET with path param).
///
/// # Errors
///
/// Returns `AppError::BadRequest` for query-building failures.
/// Returns `AppError::NotFound` if the record does not exist.
async fn handle_get_one(
    db_context: &DatabaseContext,
    select_ctx: &SelectContext,
    crud: &crate::config::types::CrudConfig,
    pk_value: &Option<String>,
    context: &RequestContext,
) -> Result<Response, AppError> {
    let pk = pk_value
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("ID parameter required".to_string()))?;
    let built = build_select_one(db_context, select_ctx, pk, context)?;
    match db_context
        .pool
        .fetch_optional_json(&built.sql, &built.params)
        .await?
    {
        Some(row) => Ok((StatusCode::OK, Json(row)).into_response()),
        None => Err(AppError::NotFound(format!(
            "{} with id '{}' not found",
            crud.table, pk
        ))),
    }
}

/// Handle create new record (POST).
///
/// # Errors
///
/// Returns `AppError::BadRequest` if request body is missing or query building fails.
/// Returns `AppError::Internal` for database failures.
async fn handle_create(
    db_context: &DatabaseContext,
    mutate_ctx: &MutationContext,
    body: &Option<Json<serde_json::Value>>,
    context: &RequestContext,
) -> Result<Response, AppError> {
    let body = body
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("Request body required".to_string()))?;
    let built = match build_insert(db_context, mutate_ctx, body, context) {
        Ok(q) => q,
        Err(e) => return Err(e),
    };

    if built.sql.contains("RETURNING") {
        let row = db_context
            .pool
            .fetch_optional_json(&built.sql, &built.params)
            .await?;
        Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({ "data": row })),
        )
            .into_response())
    } else {
        let rows_affected = db_context
            .pool
            .execute_with_params(&built.sql, &built.params)
            .await?;
        Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({ "rows_affected": rows_affected })),
        )
            .into_response())
    }
}

/// Handle update record by PK (PUT or PATCH).
///
/// # Errors
///
/// Returns `AppError::BadRequest` if request body is missing.
/// Returns `AppError::NotFound` if no matching record found.
#[allow(clippy::too_many_arguments)]
async fn handle_update(
    db_context: &DatabaseContext,
    mutate_ctx: MutationContext,
    crud: &crate::config::types::CrudConfig,
    pk_value: &Option<String>,
    body: &Option<Json<serde_json::Value>>,
    context: &RequestContext,
) -> Result<Response, AppError> {
    let pk = pk_value
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("ID parameter required".to_string()))?;
    let body = body
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("Request body required".to_string()))?;
    let built = build_update(
        db_context,
        mutate_ctx,
        pk,
        body,
        context,
        &crud.update_where_clause,
    )?;
    let rows_affected = db_context
        .pool
        .execute_with_params(&built.sql, &built.params)
        .await?;
    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "{} with id '{}' not found",
            crud.table, pk
        )));
    }
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "rows_affected": rows_affected })),
    )
        .into_response())
}

/// Handle delete record by PK (DELETE).
///
/// # Errors
///
/// Returns `AppError::BadRequest` if ID parameter is missing.
/// Returns `AppError::NotFound` if no matching record found.
async fn handle_delete(
    db_context: &DatabaseContext,
    crud: &crate::config::types::CrudConfig,
    pk_value: &Option<String>,
    context: &RequestContext,
) -> Result<Response, AppError> {
    let pk = pk_value
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("ID parameter required".to_string()))?;

    let built = build_delete(pk, db_context, context, &crud.delete_where_clause)?;
    let rows_affected = db_context
        .pool
        .execute_with_params(&built.sql, &built.params)
        .await?;
    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "{} with id '{}' not found",
            crud.table, pk
        )));
    }
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "rows_affected": rows_affected })),
    )
        .into_response())
}

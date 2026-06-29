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
use axum::response::IntoResponse;

use crate::config::types::EndpointConfig;
use crate::db::query::builders::{
    build_delete, build_insert, build_select_list, build_select_one, build_update,
};
use crate::db::query::helpers::{build_select_list_count, extract_query_params};
use crate::db::query::types::{MutationContext, SelectContext};
use crate::error::AppError;
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
) -> Result<impl IntoResponse, AppError> {
    let crud = endpoint
        .crud
        .as_ref()
        .ok_or_else(|| AppError::Internal("CRUD config missing on crud endpoint".to_string()))?;

    let select_ctx = SelectContext::from(crud);
    let mutate_ctx = MutationContext::from(crud);

    let config = state.config.read().await;
    let table_config = config
        .tables
        .iter()
        .find(|t| t.name == crud.table && t.database == crud.database)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "Table '{}' in database '{}' not found in config",
                crud.table, crud.database
            ))
        })?;

    let pool = {
        let pools = state.db_pools.read().await;
        pools
            .get(crud.database.as_str())
            .ok_or_else(|| AppError::Internal(format!("Database '{}' has no pool", crud.database)))?
            .clone()
    };
    let driver = pool.driver();

    // Extract PK from path params (e.g. {id}).
    let pk_value = path_params.as_ref().and_then(|p| p.get("id").cloned());

    match (method.as_str(), pk_value.as_deref()) {
        // GET /resources -> list
        ("GET", None) => {
            let qp = extract_query_params(&query_string);
            let built = match build_select_list(table_config, &select_ctx, &qp, driver, &context) {
                Ok(q) => q,
                Err(e) => {
                    tracing::error!("build_select_list failed: {:?}", e);
                    return Err(e);
                }
            };

            let rows = match pool.fetch_all_json(&built.sql, &built.params).await {
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

            // Build pagination metadata if pagination is configured.
            let pagination = &crud.pagination;
            let default_page_size = pagination.default_page_size;
            let max_page_size = pagination.max_page_size;
            let effective_page_size = qp
                .page_size
                .unwrap_or(default_page_size)
                .min(max_page_size)
                .max(1);
            let page = qp.page.unwrap_or(1);

            // Get total count using the same filters as the list query.
            let count_q =
                build_select_list_count(&table_config.name, driver, &qp, &select_ctx, &context);
            let total = match count_q {
                Ok(count_build) => match pool
                    .fetch_optional_json(&count_build.sql, &count_build.params)
                    .await
                {
                    Ok(Some(row)) => row.get("count").and_then(|v| v.as_u64()).unwrap_or(0),
                    _ => 0,
                },
                Err(_) => 0,
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

        // GET /resources/{id} -> get one
        ("GET", Some(pk)) => {
            let built = build_select_one(table_config, &select_ctx, pk, driver, &context)?;
            match pool.fetch_optional_json(&built.sql, &built.params).await? {
                Some(row) => Ok((StatusCode::OK, Json(row)).into_response()),
                None => Err(AppError::NotFound(format!(
                    "{} with id '{}' not found",
                    crud.table, pk
                ))),
            }
        }

        // POST /resources -> create
        ("POST", _) => {
            let body = body
                .ok_or_else(|| AppError::BadRequest("Request body required".to_string()))?
                .0;
            let built = match build_insert(table_config, &mutate_ctx, &body, driver, &context) {
                Ok(q) => q,
                Err(e) => {
                    return Err(e);
                }
            };

            if built.sql.contains("RETURNING") {
                let row = pool.fetch_optional_json(&built.sql, &built.params).await?;
                Ok((
                    StatusCode::CREATED,
                    Json(serde_json::json!({ "data": row })),
                )
                    .into_response())
            } else {
                let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;
                Ok((
                    StatusCode::CREATED,
                    Json(serde_json::json!({ "rows_affected": rows_affected })),
                )
                    .into_response())
            }
        }

        // PUT or PATCH /resources/{id} -> update
        ("PUT" | "PATCH", Some(pk)) => {
            let body = body
                .ok_or_else(|| AppError::BadRequest("Request body required".to_string()))?
                .0;
            let built = build_update(
                table_config,
                mutate_ctx,
                pk,
                &body,
                driver,
                &context,
                &crud.update_where_clause,
            )?;
            let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;
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

        // DELETE /resources/{id} -> delete
        ("DELETE", Some(pk)) => {
            let built = build_delete(
                table_config,
                pk,
                driver,
                &context,
                &crud.delete_where_clause,
            )?;
            let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;
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
        _ => Err(AppError::BadRequest("Unsupported method".to_string())),
    }
}

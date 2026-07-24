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

use crate::config::schema_registry::SchemaRegistry;
use crate::config::types::EndpointConfig;
use crate::db::query::builders::{
    build_delete, build_insert, build_select_list, build_select_one, build_update,
};
use crate::db::query::helpers::{
    build_select_list_count, extract_query_params, find_pk_column, placeholder, quote_identifier,
    validate_jsonb_body,
};
use crate::db::query::types::{MutationContext, SelectContext};
use crate::error::AppError;
use crate::handlers::common::utils::DatabaseContext;
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

    let db_ctx = state.get_db_context(&crud.database, &crud.table).await?;

    let pk_value = path_params.as_ref().and_then(|p| p.get("id").cloned());

    match (method.as_str(), pk_value.as_deref()) {
        ("GET", None) => handle_list(&db_ctx, &select_ctx, crud, &query_string, &context).await,
        ("GET", Some(_)) => handle_get_one(&db_ctx, &select_ctx, crud, &pk_value, &context).await,
        ("POST", _) => {
            let registry = state.schema_registry.read().await;
            handle_create(&db_ctx, &mutate_ctx, &body, &context, &registry).await
        }
        ("PUT" | "PATCH", Some(_)) => {
            let registry = state.schema_registry.read().await;
            handle_update(
                &db_ctx, mutate_ctx, crud, &pk_value, &body, &context, &registry,
            )
            .await
        }
        ("DELETE", Some(_)) => handle_delete(&db_ctx, crud, &pk_value, &context).await,
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
    db_ctx: &DatabaseContext,
    select_ctx: &SelectContext,
    crud: &crate::config::types::CrudConfig,
    query_string: &HashMap<String, String>,
    context: &RequestContext,
) -> Result<Response, AppError> {
    let qp = extract_query_params(query_string);
    let built = match build_select_list(db_ctx, select_ctx, &qp, context) {
        Ok(q) => q,
        Err(e) => {
            tracing::error!("build_select_list failed: {:?}", e);
            return Err(e);
        }
    };

    let rows = match db_ctx.pool.fetch_all_json(&built.sql, &built.params).await {
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
        &db_ctx.table_config.name,
        db_ctx.pool.driver(),
        &qp,
        select_ctx,
        context,
    );
    let total = match count_q {
        Ok(count_build) => match db_ctx
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
    db_ctx: &DatabaseContext,
    select_ctx: &SelectContext,
    crud: &crate::config::types::CrudConfig,
    pk_value: &Option<String>,
    context: &RequestContext,
) -> Result<Response, AppError> {
    let pk = pk_value
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("ID parameter required".to_string()))?;
    let built = build_select_one(db_ctx, select_ctx, pk, context)?;
    match db_ctx
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
/// Returns `AppError::BadRequest` if request body is missing or JSON Schema
/// validation fails. Returns `AppError::Internal` for database failures.
async fn handle_create(
    db_ctx: &DatabaseContext,
    mutate_ctx: &MutationContext,
    body: &Option<Json<serde_json::Value>>,
    context: &RequestContext,
    schema_registry: &SchemaRegistry,
) -> Result<Response, AppError> {
    let body = body
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("Request body required".to_string()))?;

    validate_jsonb_body(body, &db_ctx.table_config.name, schema_registry)?;

    let built = match build_insert(db_ctx, mutate_ctx, body, context) {
        Ok(q) => q,
        Err(e) => return Err(e),
    };

    if built.sql.contains("RETURNING") {
        let row = db_ctx
            .pool
            .fetch_optional_json(&built.sql, &built.params)
            .await?;
        Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({ "data": row })),
        )
            .into_response())
    } else {
        let _rows_affected = db_ctx
            .pool
            .execute_with_params(&built.sql, &built.params)
            .await?;

        // Backends without RETURNING support (MySQL) need a follow-up
        // query to retrieve the last inserted row so the response shape
        // stays consistent across all drivers.
        let pk_col = find_pk_column(&db_ctx.table_config)?;
        let last_id_sql = match db_ctx.pool.driver() {
            crate::config::types::DatabaseDriver::Sqlite => {
                "SELECT last_insert_rowid()".to_string()
            }
            crate::config::types::DatabaseDriver::Mysql => "SELECT LAST_INSERT_ID()".to_string(),
            crate::config::types::DatabaseDriver::Postgres => String::new(),
        };
        let row = if last_id_sql.is_empty() {
            None
        } else {
            let last_id_row = db_ctx.pool.fetch_optional_json(&last_id_sql, &[]).await?;
            if let Some(last_id) = last_id_row
                && let Some(id_val) = last_id
                    .get("last_insert_rowid()")
                    .or_else(|| last_id.get("LAST_INSERT_ID()"))
            {
                let id_str = id_val.to_string();
                db_ctx
                    .pool
                    .fetch_optional_json(
                        &format!(
                            "SELECT * FROM {} WHERE {} = {}",
                            quote_identifier(&db_ctx.table_config.name, db_ctx.pool.driver()),
                            quote_identifier(&pk_col, db_ctx.pool.driver()),
                            placeholder(db_ctx.pool.driver(), 1)
                        ),
                        &[serde_json::Value::String(id_str)],
                    )
                    .await?
            } else {
                None
            }
        };
        Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({ "data": row })),
        )
            .into_response())
    }
}

/// Handle update record by PK (PUT or PATCH).
///
/// # Errors
///
/// Returns `AppError::BadRequest` if request body is missing or JSON Schema
/// validation fails. Returns `AppError::NotFound` if no matching record found.
async fn handle_update(
    db_ctx: &DatabaseContext,
    mutate_ctx: MutationContext,
    crud: &crate::config::types::CrudConfig,
    pk_value: &Option<String>,
    body: &Option<Json<serde_json::Value>>,
    context: &RequestContext,
    schema_registry: &SchemaRegistry,
) -> Result<Response, AppError> {
    let pk = pk_value
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("ID parameter required".to_string()))?;
    let body = body
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("Request body required".to_string()))?;

    validate_jsonb_body(body, &db_ctx.table_config.name, schema_registry)?;

    let built = build_update(
        db_ctx,
        mutate_ctx,
        pk,
        body,
        context,
        &crud.update_where_clause,
    )?;
    let rows_affected = db_ctx
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
    db_ctx: &DatabaseContext,
    crud: &crate::config::types::CrudConfig,
    pk_value: &Option<String>,
    context: &RequestContext,
) -> Result<Response, AppError> {
    let pk = pk_value
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("ID parameter required".to_string()))?;

    let built = build_delete(pk, db_ctx, context, &crud.delete_where_clause)?;
    let rows_affected = db_ctx
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

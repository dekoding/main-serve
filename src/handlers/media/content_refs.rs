/// Media content reference handlers (attach/detach).
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::MediaConfig;
use crate::db::query::builders::{
    build_file_ref_delete, build_file_ref_insert, build_file_ref_max_order, build_file_ref_select,
};
use crate::error::AppError;

/// Handle content reference attach.
///
/// # Errors
///
/// Returns an `AppError::MethodNotAllowed` if content references are not enabled.
pub async fn handle_media_attach(
    id: &str,
    config: &MediaConfig,
    pool: &crate::db::pool::DatabasePool,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let content_refs = config
        .content_references
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Content references not enabled".to_string()))?;

    if !content_refs.enabled {
        return Err(AppError::MethodNotAllowed(
            "Content references are disabled".to_string(),
        ));
    }

    let entity_id = body
        .get("entity_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("entity_id is required".to_string()))?
        .to_string();

    let content_type = body
        .get("content_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("content_type is required".to_string()))?;

    let entity_id_str = entity_id.clone();
    let content_type_str = content_type.to_string();

    if let Some(allowed) = &content_refs.allowed_content_types
        && !allowed.is_empty()
        && !allowed.contains(&content_type_str)
    {
        return Err(AppError::BadRequest(format!(
            "content_type '{content_type}' is not allowed. Allowed: {allowed:?}"
        )));
    }

    let table = &content_refs.table;
    let file_id_col = &content_refs.media_id_column;
    let entity_id_col = &content_refs.entity_id_column;
    let content_type_col = &content_refs.content_type_column;
    let order_col = &content_refs.order_column;

    let built = build_file_ref_max_order(table, order_col, pool.driver());
    let max_row = pool.fetch_optional_json(&built.sql, &[]).await?;
    let max_order: i64 = max_row
        .as_ref()
        .and_then(|r| r.get("max_order").and_then(serde_json::Value::as_i64))
        .unwrap_or(0);

    let next_order = max_order + 1;

    let built = build_file_ref_insert(
        table,
        &[
            file_id_col.as_str(),
            entity_id_col.as_str(),
            content_type_col.as_str(),
            order_col.as_str(),
        ],
        pool.driver(),
    );

    pool.execute_with_params(
        &built.sql,
        &[
            id.into(),
            serde_json::Value::String(entity_id_str.clone()),
            serde_json::Value::String(content_type_str.clone()),
            serde_json::Value::Number(serde_json::Number::from(next_order)),
        ],
    )
    .await?;

    let built = build_file_ref_select(
        table,
        file_id_col,
        entity_id_col,
        content_type_col,
        pool.driver(),
    );
    let row = pool
        .fetch_optional_json(
            &built.sql,
            &[id.into(), entity_id.into(), content_type_str.into()],
        )
        .await?;

    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({ "data": row })),
    )
        .into_response())
}

/// Handle content reference detach.
///
/// # Errors
///
/// Returns an `AppError::MethodNotAllowed` if content references are not enabled.
pub async fn handle_media_detach(
    id: &str,
    config: &MediaConfig,
    pool: &crate::db::pool::DatabasePool,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let content_refs = config
        .content_references
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Content references not enabled".to_string()))?;

    if !content_refs.enabled {
        return Err(AppError::MethodNotAllowed(
            "Content references are disabled".to_string(),
        ));
    }

    let entity_id = body
        .get("entity_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("entity_id is required".to_string()))?;

    let content_type = body
        .get("content_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("content_type is required".to_string()))?;

    let table = &content_refs.table;
    let file_id_col = &content_refs.media_id_column;
    let entity_id_col = &content_refs.entity_id_column;
    let content_type_col = &content_refs.content_type_column;

    let built = build_file_ref_delete(
        table,
        file_id_col,
        entity_id_col,
        content_type_col,
        pool.driver(),
    );
    let rows_affected = pool
        .execute_with_params(
            &built.sql,
            &[
                id.into(),
                serde_json::Value::String(entity_id.to_string()),
                serde_json::Value::String(content_type.to_string()),
            ],
        )
        .await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "No content reference found for media id '{id}', entity '{entity_id}', type '{content_type}'"
        )));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": format!(
                "Detached media '{}' from entity '{}:{}'",
                id, entity_id, content_type
            ),
            "rows_affected": rows_affected,
        })),
    )
        .into_response())
}

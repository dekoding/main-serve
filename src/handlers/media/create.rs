/// Media create handler.
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::db::query::builders::build_insert;
use crate::db::query::types::MutationContext;
use crate::error::AppError;
use crate::handlers::common::utils::{DatabaseContext, HandlerContext};
use crate::middleware::auth::extractor::RequestContext;

pub async fn handle_media_create(
    db_ctx: &DatabaseContext,
    handler_ctx: &HandlerContext<'_>,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let user_id = handler_ctx.extract_user_id().await?;

    let writable_columns = db_ctx
        .table_config
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect::<Vec<_>>();
    let mut body_map = serde_json::Map::new();

    if let Some(obj) = body.as_object() {
        for (k, v) in obj {
            if writable_columns.contains(&k.to_string()) {
                body_map.insert(k.clone(), v.clone());
            }
        }
    }

    if writable_columns.contains(&"uploader_id".to_string()) {
        body_map.insert(
            "uploader_id".to_string(),
            serde_json::Value::String(user_id),
        );
    }

    if writable_columns.contains(&"created_at".to_string()) {
        body_map.insert(
            "created_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
    }

    let json_body = serde_json::Value::Object(body_map);
    let built = build_insert(
        db_ctx,
        &MutationContext::default(),
        &json_body,
        &RequestContext::default(),
    )?;

    if built.sql.contains("RETURNING") {
        let row = db_ctx
            .pool
            .fetch_optional_json(&built.sql, &built.params)
            .await?;
        Ok((
            StatusCode::CREATED,
            axum::Json(serde_json::json!({ "data": row })),
        )
            .into_response())
    } else {
        let rows_affected = db_ctx
            .pool
            .execute_with_params(&built.sql, &built.params)
            .await?;
        Ok((
            StatusCode::CREATED,
            axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
        )
            .into_response())
    }
}

/// Media update handler.
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::MediaConfig;
use crate::db::query::builders::build_update;
use crate::db::query::select_one::build_select_by_id;
use crate::db::query::types::MutationContext;
use crate::error::AppError;
use crate::handlers::common::utils::{DatabaseContext, HandlerContext, filter_writable_body};
use crate::middleware::auth::extractor::RequestContext;

// collapsible_if suppressed: early returns improve readability for ownership checks.
#[allow(clippy::collapsible_if)]
pub async fn handle_media_update(
    handler_ctx: &HandlerContext<'_>,
    id: &str,
    config: &MediaConfig,
    db_ctx: &DatabaseContext,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let auth_info = handler_ctx.extract_auth_info().await?;
    if let Some(user_scope) = &config.user_scope {
        if user_scope.enabled && !user_scope.allow_cross_user_browse {
            let current_user = &auth_info.subject;

            let built = build_select_by_id(&config.table, &["uploader_id"], db_ctx.pool.driver())
                .map_err(|e| AppError::Internal(format!("Failed to build query: {e}")))?;
            let row = db_ctx
                .pool
                .fetch_optional_json(&built.sql, &[id.into()])
                .await?;
            if let Some(row) = row {
                if let Some(uploader_id) = row.get("uploader_id").and_then(|v| v.as_str()) {
                    if uploader_id != current_user {
                        return Err(AppError::Forbidden(
                            "Cannot update media owned by another user".to_string(),
                        ));
                    }
                }
            }
        }
    }

    let writable_columns: Vec<String> = db_ctx
        .table_config
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect();

    let mut body_map = filter_writable_body(body, &writable_columns);

    if db_ctx
        .table_config
        .columns
        .iter()
        .any(|c| c.name == "updated_at")
    {
        body_map.insert(
            "updated_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
    }
    body_map.insert(
        "last_modified_by".to_string(),
        serde_json::Value::String(auth_info.subject),
    );

    let built = build_update(
        db_ctx,
        MutationContext::default(),
        id,
        &serde_json::Value::Object(body_map),
        &RequestContext::default(),
        &None,
    )?;
    let rows_affected = db_ctx
        .pool
        .execute_with_params(&built.sql, &built.params)
        .await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "Media item with id '{}' not found",
            id
        )));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
    )
        .into_response())
}

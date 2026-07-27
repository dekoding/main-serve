use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::db::query::builders::{build_select_one, build_update};
use crate::db::query::types::{MutationContext, SelectContext};
use crate::error::AppError;
use crate::handlers::common::utils::filter_writable_body;
use crate::handlers::file_store::{
    FileStoreContext, check_file_store_ownership, is_field_writable, user_roles,
};
use crate::middleware::auth::extractor::RequestContext;

/// Handle updating a file store entry.
pub async fn handle_file_store_update(ctx: &FileStoreContext<'_>) -> Result<Response, AppError> {
    let pool = &ctx.db_ctx.pool;
    let table_config = &ctx.db_ctx.table_config;
    let driver = &ctx.db_ctx.pool.driver();
    let auth_info = ctx.handler_ctx.extract_auth_info().await?;

    if ctx.config.ownership.is_some() && !ctx.handler_ctx.endpoint.roles.is_admin(&auth_info.role) {
        check_file_store_ownership(ctx.handler_ctx, ctx.config, ctx.require_id()?, driver).await?;
    }

    // Check field write permissions
    if let Some(ref permissions) = ctx.config.field_permissions {
        let user_role = auth_info.role;
        let user_roles = user_roles(&user_role);
        if let Some(obj) = ctx.body.and_then(|v| v.as_object()) {
            for (k, _) in obj {
                if table_config.columns.iter().any(|c| c.name == *k)
                    && k != "updated_at"
                    && !is_field_writable(k, &user_roles, permissions)
                {
                    return Err(AppError::Forbidden(format!(
                        "Field '{}' is not writable by the current user's roles",
                        k
                    )));
                }
            }
        }
    }

    let writable_columns: Vec<String> = table_config
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect();
    let body_value = ctx
        .body
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    let mut body_map = filter_writable_body(&body_value, &writable_columns);
    body_map.insert(
        "updated_at".to_string(),
        serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
    );

    let mutate_ctx = MutationContext {
        writable_fields: vec!["*".to_string()],
        ..MutationContext::default()
    };
    let built = build_update(
        ctx.db_ctx,
        mutate_ctx,
        ctx.require_id()?,
        &serde_json::Value::Object(body_map),
        &RequestContext::default(),
        &None,
    )?;
    let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "File entry with id '{}' not found",
            ctx.require_id()?
        )));
    }

    // Return the updated row
    let built = build_select_one(
        ctx.db_ctx,
        &SelectContext::permissive(),
        ctx.require_id()?,
        &RequestContext::default(),
    )?;
    let row = pool.fetch_optional_json(&built.sql, &built.params).await?;

    let response = match row {
        Some(row) => axum::Json(row),
        None => axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
    };

    Ok((StatusCode::OK, response).into_response())
}

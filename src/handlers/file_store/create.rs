//! Create new file store entries.
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::db::query::builders::build_insert;
use crate::db::query::helpers::validate_jsonb_body;
use crate::db::query::types::MutationContext;
use crate::error::AppError;
use crate::handlers::common::utils::filter_writable_body;
use crate::handlers::file_store::{FileStoreContext, is_field_writable, user_roles};
use crate::middleware::auth::extractor::RequestContext;

/// Handle creating a file store entry.
///
/// # Errors
///
/// Returns an error on authentication failure, field permission violations,
/// or database errors.
pub async fn handle_file_store_create(ctx: &FileStoreContext<'_>) -> Result<Response, AppError> {
    let pool = &ctx.db_ctx.pool;
    let table_config = &ctx.db_ctx.table_config;
    let user_id = ctx.handler_ctx.extract_user_id().await?;

    // Check field write permissions
    if let Some(ref permissions) = ctx.config.field_permissions {
        let auth_info = ctx.handler_ctx.extract_auth_info().await?;
        let user_role = auth_info.role;
        let user_roles = user_roles(&user_role);
        if let Some(obj) = ctx.body.and_then(|v| v.as_object()) {
            for (k, _) in obj {
                if table_config.columns.iter().any(|c| c.name == *k)
                    && !is_field_writable(k, &user_roles, permissions)
                {
                    return Err(AppError::Forbidden(format!(
                        "Field '{k}' is not writable by the current user's roles"
                    )));
                }
            }
        }
    }

    let writable_columns = table_config
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect::<Vec<_>>();
    let body_value = ctx
        .body
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    let registry = ctx.handler_ctx.state.schema_registry.read().await;
    validate_jsonb_body(&body_value, &ctx.db_ctx.table_config.name, &registry)?;
    let mut body_map = filter_writable_body(&body_value, &writable_columns);

    if ctx.config.ownership.is_some() {
        let owner_col = ctx
            .config
            .ownership
            .as_ref()
            .map_or("owner_id", |o| o.owner_column.as_str());
        body_map.insert(
            owner_col.to_string(),
            serde_json::Value::String(user_id.clone()),
        );
    }

    if writable_columns.contains(&"created_at".to_string()) {
        body_map.insert(
            "created_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
    }

    if writable_columns.contains(&"updated_at".to_string()) {
        body_map.insert(
            "updated_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
    }

    let json_body = serde_json::Value::Object(body_map);
    let built = build_insert(
        ctx.db_ctx,
        &MutationContext {
            writable_fields: vec!["*".to_string()],
            ..MutationContext::default()
        },
        &json_body,
        &RequestContext::default(),
    )?;

    if built.sql.contains("RETURNING") {
        let mut row = pool.fetch_optional_json(&built.sql, &built.params).await?;
        // Add owner column to response if ownership is configured
        if let Some(serde_json::Value::Object(ref mut obj)) = row
            && let Some(ownership) = &ctx.config.ownership
        {
            let owner_col = ownership.owner_column.as_str();
            obj.insert(
                owner_col.to_string(),
                serde_json::Value::String(user_id.clone()),
            );
        }
        // Convert id to string if it's a number
        if let Some(serde_json::Value::Object(ref mut obj)) = row
            && let Some(id_val) = obj.get("id")
        {
            let id_str = id_val
                .as_i64()
                .map_or_else(|| id_val.to_string(), |i| i.to_string());
            obj.insert("id".to_string(), serde_json::Value::String(id_str));
        }
        Ok((
            StatusCode::CREATED,
            axum::Json(serde_json::json!({ "data": row })),
        )
            .into_response())
    } else {
        let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;
        Ok((
            StatusCode::CREATED,
            axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
        )
            .into_response())
    }
}

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::FileStoreConfig;
use crate::db::query::builders::build_select_one;
use crate::db::query::types::SelectContext;
use crate::error::AppError;
use crate::handlers::common::utils::HandlerContext;
use crate::handlers::file_store::{
    check_file_store_ownership, is_field_readable_by_role, user_roles,
};
use crate::middleware::auth::extractor::RequestContext;

/// Handle getting a single file store entry.
///
/// # Errors
///
/// Returns an error on authentication failure, ownership violations, or
/// database errors.
pub async fn handle_file_store_get_one(
    handler_ctx: &HandlerContext<'_>,
    db_ctx: &crate::handlers::common::utils::DatabaseContext,
    config: &FileStoreConfig,
    id: &str,
) -> Result<Response, AppError> {
    // Check ownership if configured
    if let Some(ownership) = &config.ownership {
        let auth_info = handler_ctx.extract_auth_info().await?;
        let is_admin = handler_ctx.endpoint.roles.is_admin(&auth_info.role);
        if ownership.admin_override && !is_admin {
            check_file_store_ownership(handler_ctx, config, id, &db_ctx.pool.driver()).await?;
        }
    }

    let built = build_select_one(
        db_ctx,
        &SelectContext::permissive(),
        id,
        &RequestContext::default(),
    )?;
    match db_ctx
        .pool
        .fetch_optional_json(&built.sql, &built.params)
        .await?
    {
        Some(mut row) => {
            if let Some(permissions) = &config.field_permissions {
                let auth_info = handler_ctx.extract_auth_info().await?;
                let user_role = auth_info.role;
                let user_roles = user_roles(&user_role);
                let mut filtered = serde_json::Map::new();
                if let Some(obj) = row.as_object_mut() {
                    let keys: Vec<String> = obj.keys().cloned().collect();
                    for key in keys {
                        if is_field_readable_by_role(&key, &user_roles, permissions)
                            && let Some(value) = obj.remove(&key)
                        {
                            filtered.insert(key, value);
                        }
                    }
                }
                row = serde_json::Value::Object(filtered);
            }
            // Convert id to string if it's a number (consistent with CREATE response)
            if let serde_json::Value::Object(ref mut obj) = row
                && let Some(id_val) = obj.get("id")
            {
                let id_str = id_val
                    .as_i64()
                    .map_or_else(|| id_val.to_string(), |i| i.to_string());
                obj.insert("id".to_string(), serde_json::Value::String(id_str));
            }
            Ok((StatusCode::OK, axum::Json(row)).into_response())
        }
        None => Err(AppError::NotFound(format!(
            "File entry with id '{id}' not found"
        ))),
    }
}

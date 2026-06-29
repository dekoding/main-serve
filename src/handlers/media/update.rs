/// Media update handler.
use std::collections::HashMap;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::{DatabaseDriver, EndpointConfig, MediaConfig, TableConfig};
use crate::db::query::builders::build_update;
use crate::db::query::select_one::build_select_by_id;
use crate::db::query::types::MutationContext;
use crate::error::AppError;
use crate::middleware::auth::extractor::RequestContext;

// collapsible_if suppressed: early returns improve readability for ownership checks.
// These functions need access to state, configs, headers, query params, and body.
#[allow(clippy::collapsible_if)]
#[allow(clippy::too_many_arguments)]
pub async fn handle_media_update(
    state: &crate::server::state::AppState,
    id: &str,
    config: &MediaConfig,
    table_config: &TableConfig,
    driver: DatabaseDriver,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    body: &serde_json::Value,
    query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    if let Some(user_scope) = &config.user_scope {
        if user_scope.enabled && !user_scope.allow_cross_user_browse {
            let auth_info = extract_auth_info(state, endpoint, headers, query_params).await?;
            let current_user = auth_info.subject;

            let pool = {
                let pools = state.db_pools.read().await;
                pools
                    .get(&config.database)
                    .ok_or_else(|| {
                        AppError::Internal(format!("Database '{}' has no pool", config.database))
                    })?
                    .clone()
            };

            let built = build_select_by_id(&config.table, &["uploader_id"], driver)
                .map_err(|e| AppError::Internal(format!("Failed to build query: {e}")))?;
            let row = pool.fetch_optional_json(&built.sql, &[id.into()]).await?;
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

    let pool = {
        let pools = state.db_pools.read().await;
        pools
            .get(&config.database)
            .ok_or_else(|| {
                AppError::Internal(format!("Database '{}' has no pool", config.database))
            })?
            .clone()
    };

    let auth_info = extract_auth_info(state, endpoint, headers, query_params).await?;
    let mut body_map = serde_json::Map::new();

    if let Some(obj) = body.as_object() {
        for (k, v) in obj {
            if table_config.columns.iter().any(|c| c.name == *k) {
                body_map.insert(k.clone(), v.clone());
            }
        }
    }

    if table_config.columns.iter().any(|c| c.name == "updated_at") {
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
        table_config,
        MutationContext::default(),
        id,
        &serde_json::Value::Object(body_map),
        driver,
        &RequestContext::default(),
        &None,
    )?;
    let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "Media item with id '{}' not found",
            &id
        )));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
    )
        .into_response())
}

async fn extract_auth_info(
    state: &crate::server::state::AppState,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<crate::middleware::auth::extractor::AuthInfo, AppError> {
    if endpoint.auth == "none" {
        return Ok(crate::middleware::auth::extractor::AuthInfo::default());
    }
    let auth_config = state.config.read().await.auth.clone();
    crate::middleware::auth::validate::authenticate::<crate::server::state::InMemoryRevocationStore>(
        &endpoint.auth,
        &auth_config,
        headers,
        query_params,
        None,
    )
    .await
}

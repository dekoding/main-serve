/// Media delete handlers.
use std::path::Path;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::{EndpointConfig, MediaConfig, TableConfig};
use crate::db::migration::quote_object_name;
use crate::db::query::builders::build_delete;
use crate::error::AppError;
use crate::handlers::media::trash::handle_media_trash_delete;
use crate::middleware::auth::extractor::RequestContext;
use crate::storage::Storage;

// These functions need access to configs, state, storage, and database pool.
#[allow(clippy::too_many_arguments)]
pub async fn handle_media_delete(
    table_config: &TableConfig,
    state: &crate::server::state::AppState,
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    pool: &crate::db::pool::DatabasePool,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &std::collections::HashMap<String, String>,
) -> Result<Response, AppError> {
    let trash_enabled = config.trash.as_ref().is_some_and(|t| t.enabled);

    if trash_enabled {
        handle_media_trash_delete(
            state,
            id,
            config,
            storage,
            root,
            pool,
            endpoint,
            headers,
            query_params,
        )
        .await
    } else {
        delete_media_permanently(id, config, storage, root, pool, table_config).await
    }
}

// collapsible_if suppressed: early return pattern would obscure the delete logic.
pub async fn delete_media_permanently(
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    pool: &crate::db::pool::DatabasePool,
    table_config: &TableConfig,
) -> Result<Response, AppError> {
    let driver = pool.driver();
    let path_check = format!(
        "SELECT file_path FROM {} WHERE id = $1",
        quote_object_name(&config.table, driver)
    );
    let row = pool.fetch_optional_json(&path_check, &[id.into()]).await?;

    if let Some(row) = row
        && let Some(file_path) = row.get("file_path").and_then(|v| v.as_str())
    {
        let file_path_buf = root.join(file_path);
        if storage.exists(&file_path_buf).await {
            storage
                .delete(&file_path_buf)
                .await
                .map_err(|e| AppError::FileOperation(format!("Failed to delete file: {e}")))?;
        }
    }

    let built = build_delete(table_config, id, driver, &RequestContext::default(), &None)?;
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

/// Media library handler.
pub mod content_refs;
pub mod create;
pub mod delete;
pub mod list;
pub mod move_rename;
pub mod resize;
pub mod sharing;
pub mod trash;
pub mod update;
pub mod upload;

use std::collections::HashMap;
use std::path::PathBuf;

use axum::extract::{MatchedPath, Query, State};
use axum::http::{HeaderMap, Method, Uri};
use axum::response::Response;

use crate::config::types::EndpointConfig;
use crate::error::AppError;
use crate::handlers::common::helpers::extract_id;
use crate::handlers::common::store::resolve_store;
use crate::handlers::common::utils::get_db_context;
use crate::handlers::media::content_refs::{handle_media_attach, handle_media_detach};
use crate::handlers::media::create::handle_media_create;
use crate::handlers::media::delete::handle_media_delete;
use crate::handlers::media::list::{handle_media_get, handle_media_list};
use crate::handlers::media::move_rename::{handle_media_move, handle_media_rename};
use crate::handlers::media::resize::{handle_media_resize, handle_media_thumbnail};
use crate::handlers::media::sharing::handle_media_share_get;
use crate::handlers::media::trash::handle_media_trash;
use crate::handlers::media::update::handle_media_update;
use crate::handlers::media::upload::handle_media_upload;
use crate::server::state::AppState;

/// Route handler for media library endpoints.
pub async fn handle_media_route(
    state: State<AppState>,
    matched_path: MatchedPath,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    query: Query<HashMap<String, String>>,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config_for_method(path_str, &method)
        .await
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    let body_value = if body.is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
    };

    handle_media(
        &state,
        method,
        uri,
        &endpoint,
        headers,
        query.0,
        &body_value,
    )
    .await
}

/// Route handler for media upload (multipart POST).
pub async fn handle_media_upload_route(
    state: axum::extract::State<AppState>,
    matched_path: axum::extract::MatchedPath,
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
    query: axum::extract::Query<HashMap<String, String>>,
    multipart: axum::extract::Multipart,
) -> Result<Response, AppError> {
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config_for_method(path_str, &method)
        .await
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    if method != axum::http::Method::POST
        && method != axum::http::Method::PUT
        && method != axum::http::Method::PATCH
    {
        return Err(AppError::MethodNotAllowed(
            "Method not allowed for media upload".to_string(),
        ));
    }

    let config = endpoint
        .media
        .as_ref()
        .ok_or_else(|| AppError::NotFound("Media config not found".to_string()))?;

    if config.upload.as_ref().is_some_and(|u| u.max_size == 0) {
        return Err(AppError::MethodNotAllowed(
            "Media upload is not enabled".to_string(),
        ));
    }

    handle_media_upload(state, multipart, &endpoint, &uri, &headers, query.0).await
}

/// Core media handler logic - dispatches to specific route handlers.
#[allow(clippy::collapsible_if)]
pub(crate) async fn handle_media(
    state: &AppState,
    method: axum::http::Method,
    uri: axum::http::Uri,
    endpoint: &EndpointConfig,
    headers: axum::http::HeaderMap,
    query_params: HashMap<String, String>,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let config = endpoint
        .media
        .as_ref()
        .ok_or_else(|| AppError::NotFound("Media config not found".to_string()))?;

    let path = uri.path().to_string();

    let storage = state
        .get_store(&config.storage)
        .ok_or_else(|| AppError::Internal(format!("Store '{}' not found", config.storage)))?;

    let root = storage
        .root_path()
        .unwrap_or(PathBuf::from(&config.storage));

    let (pool, table_config, driver) =
        get_db_context(state, config.database.clone(), config.table.clone()).await?;

    // Dispatch special-purpose routes (trash, sharing, resize, thumbnail, attach, detach, move, rename)
    if path.starts_with("/_main-serve/media/trash") {
        let (storage, root) = resolve_store(state, &config.storage)?;
        return handle_media_trash(
            state,
            method,
            &path,
            config,
            &*storage,
            &root,
            &pool,
            &table_config,
            endpoint,
            &headers,
            &query_params,
        )
        .await;
    }

    if path.starts_with("/shared/") {
        let token = path.strip_prefix("/shared/").unwrap_or("");
        let (storage, root) = resolve_store(state, &config.storage)?;
        return handle_media_share_get(token, config, &*storage, &root).await;
    }

    if path.contains("/resize") {
        if let Some(id) = extract_id(&path) {
            let (storage, root) = resolve_store(state, &config.storage)?;
            return handle_media_resize(&*storage, &root, &id, config, &query_params, &pool).await;
        }
    }

    if path.contains("/thumbnail") {
        if let Some(id) = extract_id(&path) {
            let (storage, root) = resolve_store(state, &config.storage)?;
            return handle_media_thumbnail(&*storage, &root, &id, config, &pool).await;
        }
    }

    if path.contains("/attach") {
        if let Some(id) = extract_id(&path) {
            return handle_media_attach(&id, config, &pool, body).await;
        }
    }

    if path.contains("/detach") {
        if let Some(id) = extract_id(&path) {
            return handle_media_detach(&id, config, &pool, body).await;
        }
    }

    if path.contains("/move") {
        if let Some(id) = extract_id(&path) {
            let (storage, root) = resolve_store(state, &config.storage)?;
            return handle_media_move(&id, config, &pool, &*storage, &root, body).await;
        }
    }

    if path.contains("/rename") {
        if let Some(id) = extract_id(&path) {
            let (storage, root) = resolve_store(state, &config.storage)?;
            return handle_media_rename(&id, config, &pool, &*storage, &root, body).await;
        }
    }

    // Determine the base path for this endpoint (strip parameterized segments).
    let base_path = endpoint
        .path
        .strip_suffix("/{id}")
        .unwrap_or(&endpoint.path)
        .strip_suffix("/*")
        .unwrap_or(&endpoint.path);

    // Normalize: strip trailing slash for consistent comparison.
    let path_normalized = path.trim_end_matches('/');
    let base_normalized = base_path.trim_end_matches('/');

    let is_list_request = path_normalized.is_empty()
        || path_normalized == "/"
        || path_normalized == base_normalized
        || path == format!("{}/", base_normalized);

    match method {
        axum::http::Method::GET => {
            if is_list_request {
                handle_media_list(&pool, config, &table_config, &query_params).await
            } else {
                match extract_id(&path) {
                    Some(id) => handle_media_get(&pool, &table_config, &id).await,
                    None => Err(AppError::BadRequest("Media ID required".to_string())),
                }
            }
        }
        axum::http::Method::POST => {
            if path.is_empty() || path == "/" {
                handle_media_create(
                    &pool,
                    &table_config,
                    endpoint,
                    &headers,
                    body,
                    state,
                    &query_params,
                )
                .await
            } else {
                Err(AppError::MethodNotAllowed(
                    "POST not allowed on this path".to_string(),
                ))
            }
        }
        axum::http::Method::PATCH => match extract_id(&path) {
            Some(id) => {
                handle_media_update(
                    state,
                    &id,
                    config,
                    &table_config,
                    driver,
                    endpoint,
                    &headers,
                    body,
                    &query_params,
                )
                .await
            }
            None => Err(AppError::BadRequest("Media ID required".to_string())),
        },
        axum::http::Method::DELETE => match extract_id(&path) {
            Some(id) => {
                handle_media_delete(
                    &table_config,
                    state,
                    &id,
                    config,
                    &*storage,
                    &root,
                    &pool,
                    endpoint,
                    &headers,
                    &query_params,
                )
                .await
            }
            None => Err(AppError::BadRequest("Media ID required".to_string())),
        },
        _ => Err(AppError::MethodNotAllowed(
            "Method not allowed for media endpoint".to_string(),
        )),
    }
}

//! Single-page application host endpoint handler.
//!
//! Serves static files with SPA-style fallback: non-existent paths return
//! the index file with the configured fallback status code. Read-only (GET/HEAD only).
use std::path::Path;

use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use http::header;

use crate::config::types::HttpMethod;
use crate::config::types::mime_from_path;
use crate::config::types::{EndpointConfig, SpaHostConfig};
use crate::error::AppError;
use crate::handlers::common::path::extract_relative_path;
use crate::handlers::common::store::resolve_store;
use crate::handlers::common::utils::{
    apply_cache_control, apply_content_length, apply_content_type,
};
use crate::server::state::AppState;
use crate::storage::Storage;

/// Route handler for SPA hosting.
///
/// # Errors
///
/// Returns an `AppError::NotFound` if the endpoint is not found.
pub async fn handle_spa_host_route(
    state: axum::extract::State<AppState>,
    matched_path: axum::extract::MatchedPath,
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
) -> Result<Response, AppError> {
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config_for_method(path_str, &method)
        .await
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    handle_spa_host(&state, method, uri, &endpoint, headers).await
}

/// Core SPA host handler logic.
pub(crate) async fn handle_spa_host(
    state: &AppState,
    method: axum::http::Method,
    uri: axum::http::Uri,
    endpoint: &EndpointConfig,
    headers: axum::http::HeaderMap,
) -> Result<Response, AppError> {
    let config = endpoint
        .spa_host
        .as_ref()
        .ok_or_else(|| AppError::NotFound("SPA host config not found".to_string()))?;

    // HEAD support check
    if method == axum::http::Method::HEAD {
        if !config.head_support {
            let allowed: Vec<HttpMethod> = endpoint
                .methods
                .iter()
                .copied()
                .filter(|m| *m != HttpMethod::Head)
                .collect();
            return Err(AppError::MethodNotAllowed {
                message: "HEAD requests are not supported for this endpoint".to_string(),
                allowed,
            });
        }
        return handle_spa_head(state, uri, config, &headers).await;
    }

    let (storage, root) = resolve_store(state, &config.storage)?;

    let request_path = uri.path();
    let relative = extract_relative_path(request_path, &endpoint.path)?;

    let resolved = if relative.is_empty() {
        root.clone()
    } else {
        let decoded = percent_encoding::percent_decode_str(&relative)
            .decode_utf8()
            .map_err(|_| AppError::BadRequest("Invalid UTF-8 in path".to_string()))?;
        root.join(decoded.as_ref())
    };

    // Check if the path points to a directory - try index file
    let meta = storage.metadata(&resolved).await.ok();
    if meta.as_ref().is_some_and(|m| !m.is_file) {
        let index_path = resolved.join(&config.index);
        if storage.metadata(&index_path).await.is_ok_and(|m| m.is_file) {
            return serve_spa_file(&*storage, &index_path, config, &headers).await;
        }
        // Directory not found, fall through to SPA fallback
    }

    // Try to serve the file
    match serve_spa_file(&*storage, &resolved, config, &headers).await {
        Ok(response) => Ok(response),
        Err(AppError::NotFound(_)) => serve_spa_fallback(&*storage, &root, config, &headers).await,
        Err(e) => Err(e),
    }
}

/// Handle HEAD requests - return headers only, no body.
pub(crate) async fn handle_spa_head(
    state: &AppState,
    uri: axum::http::Uri,
    config: &SpaHostConfig,
    headers: &axum::http::HeaderMap,
) -> Result<Response, AppError> {
    let (storage, root) = resolve_store(state, &config.storage)?;

    let request_path = uri.path();
    let relative = request_path.trim_start_matches('/');

    let resolved = if relative.is_empty() {
        root.clone()
    } else {
        root.join(relative)
    };

    if let Ok(meta) = storage.metadata(&resolved).await {
        let content_type = mime_from_path(&resolved);

        let mut response = Response::new(axum::body::Body::empty());
        *response.status_mut() = StatusCode::OK;
        apply_content_type(&mut response, content_type);
        apply_cache_control(
            &mut response,
            config.cache_max_age,
            Some(&resolved),
            &config.cache_rules,
        );
        apply_content_length(&mut response, &meta.size.to_string());

        if config.etag {
            let etag_value = format!("\"{}-{}\"", resolved.display(), meta.size);
            response.headers_mut().insert(
                header::ETAG,
                HeaderValue::from_str(&etag_value).unwrap_or(HeaderValue::from_static("\"none\"")),
            );

            if let Some(if_none_match) = headers.get(header::IF_NONE_MATCH) {
                let etag_match = if_none_match
                    .to_str()
                    .ok()
                    .is_some_and(|s| s.trim() == etag_value.trim());
                if etag_match {
                    *response.status_mut() = StatusCode::NOT_MODIFIED;
                    response.headers_mut().insert(
                        header::ETAG,
                        HeaderValue::from_str(&etag_value)
                            .unwrap_or(HeaderValue::from_static("\"none\"")),
                    );
                }
            }
        }

        Ok(response)
    } else {
        let index_path = root.join(&config.index);
        match storage.metadata(&index_path).await {
            Ok(meta) => {
                let content_type = mime_from_path(&index_path);
                let status = StatusCode::from_u16(config.fallback_status).unwrap_or(StatusCode::OK);
                let mut response = Response::new(axum::body::Body::empty());
                *response.status_mut() = status;
                apply_content_type(&mut response, content_type);
                apply_cache_control(
                    &mut response,
                    config.cache_max_age,
                    Some(&index_path),
                    &config.cache_rules,
                );
                apply_content_length(&mut response, &meta.size.to_string());
                Ok(response)
            }
            Err(_) => Err(AppError::NotFound(format!(
                "Index file '{}' not found for SPA HEAD fallback",
                config.index
            ))),
        }
    }
}

/// Serve a file from the SPA store with appropriate headers.
async fn serve_spa_file(
    storage: &dyn Storage,
    path: &Path,
    config: &SpaHostConfig,
    headers: &axum::http::HeaderMap,
) -> Result<Response, AppError> {
    let meta = storage
        .metadata(path)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let content_type = mime_from_path(path);

    let contents = storage
        .read(path)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let mut response = (StatusCode::OK, contents).into_response();
    apply_content_type(&mut response, content_type);
    apply_cache_control(
        &mut response,
        config.cache_max_age,
        Some(path),
        &config.cache_rules,
    );
    apply_content_length(&mut response, &meta.size.to_string());

    let resp_headers = response.headers_mut();

    if config.etag {
        let etag_value = format!("\"{}-{}\"", path.display(), meta.size);
        resp_headers.insert(
            header::ETAG,
            HeaderValue::from_str(&etag_value).unwrap_or(HeaderValue::from_static("\"none\"")),
        );

        if let Some(if_none_match) = headers.get(header::IF_NONE_MATCH) {
            let etag_match = if_none_match
                .to_str()
                .ok()
                .is_some_and(|s| s.trim() == etag_value.trim());
            if etag_match {
                *response.status_mut() = StatusCode::NOT_MODIFIED;
                *response.headers_mut() = std::mem::take(response.headers_mut());
                response.headers_mut().insert(
                    header::ETAG,
                    HeaderValue::from_str(&etag_value)
                        .unwrap_or(HeaderValue::from_static("\"none\"")),
                );
                return Ok(response);
            }
        }
    }

    Ok(response)
}

/// Serve the SPA fallback (index file) with the configured fallback status.
async fn serve_spa_fallback(
    storage: &dyn Storage,
    root: &Path,
    config: &SpaHostConfig,
    _headers: &axum::http::HeaderMap,
) -> Result<Response, AppError> {
    let index_path = root.join(&config.index);

    match storage.metadata(&index_path).await {
        Ok(meta) => {
            let content_type = mime_from_path(&index_path);
            let contents = storage.read(&index_path).await.map_err(|_| {
                AppError::NotFound(format!("Index file not found: {}", index_path.display()))
            })?;

            let status = StatusCode::from_u16(config.fallback_status).unwrap_or(StatusCode::OK);
            let mut response = (status, contents).into_response();
            apply_content_type(&mut response, content_type);
            apply_cache_control(
                &mut response,
                config.cache_max_age,
                Some(&index_path),
                &config.cache_rules,
            );
            apply_content_length(&mut response, &meta.size.to_string());

            if config.etag {
                let etag_value = format!("\"{}-{}\"", index_path.display(), meta.size);
                response.headers_mut().insert(
                    header::ETAG,
                    HeaderValue::from_str(&etag_value)
                        .unwrap_or(HeaderValue::from_static("\"none\"")),
                );
            }

            Ok(response)
        }
        Err(_) => Err(AppError::NotFound(format!(
            "Index file '{}' not found for SPA fallback",
            config.index
        ))),
    }
}

/// Return 405 Method Not Allowed for SPA host (read-only endpoints).
///
/// Registered as the route handler for POST/PUT/PATCH/DELETE methods when the
/// SPA host only supports GET, HEAD, and OPTIONS.
///
/// # Errors
///
/// Always returns an `AppError::MethodNotAllowed` with the SPA host's allowed
/// methods (filtered from the configured methods to only include GET, HEAD, OPTIONS).
pub async fn handle_spa_host_method_not_allowed(
    state: axum::extract::State<AppState>,
    matched_path: axum::extract::MatchedPath,
) -> Result<axum::response::Response, AppError> {
    let path_str = matched_path.as_str();
    let endpoint = state
        .endpoint_configs
        .read()
        .await
        .values()
        .find(|e| e.path == path_str)
        .cloned();

    let allowed: Vec<HttpMethod> = match endpoint {
        Some(ep) => ep
            .methods
            .into_iter()
            .filter(|m| {
                !matches!(
                    m,
                    HttpMethod::Post | HttpMethod::Put | HttpMethod::Patch | HttpMethod::Delete
                )
            })
            .collect(),
        None => vec![HttpMethod::Get, HttpMethod::Head, HttpMethod::Options],
    };

    Err(AppError::MethodNotAllowed {
        message: "Method Not Allowed for SPA host endpoint".to_string(),
        allowed,
    })
}

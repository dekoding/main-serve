use axum::extract::Query;
use axum::extract::State;
use axum::http::Uri;
use axum::http::{Method, StatusCode, header};
use axum::response::IntoResponse;
use axum::response::Response;
use http::HeaderValue;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::types::EndpointConfig;
use crate::config::types::StaticFilesConfig;
use crate::error::AppError;
use crate::handlers::static_files::delete::handle_file_delete;
use crate::middleware::auth::extractor::AuthInfo;
use crate::middleware::cors;
use crate::server::state::AppState;
use crate::storage::Storage;

/// Context for handling static file GET requests.
pub(crate) struct StaticGetContext<'a> {
    pub config: &'a StaticFilesConfig,
    pub relative: &'a str,
    pub root: &'a Path,
    pub request_path: &'a str,
    pub query: Option<&'a Query<HashMap<String, String>>>,
    pub headers: &'a axum::http::HeaderMap,
    pub storage: Arc<dyn Storage>,
}

/// Extract the relative file path from a request URI and endpoint path.
///
/// Strips the endpoint's base path prefix (handling `/*` and `{*rest}`
/// wildcards) from the request path and returns the remaining segment.
#[must_use]
pub fn extract_relative_path(request_path: &str, endpoint_path: &str) -> String {
    let ep_path = endpoint_path
        .trim_end_matches("/*")
        .trim_end_matches("{*rest}");
    request_path
        .strip_prefix(ep_path)
        .unwrap_or(request_path)
        .trim_start_matches('/')
        .to_string()
}

/// Extract authentication info from request.
///
/// # Errors
///
/// Returns `AppError::Auth` if authentication fails.
/// Returns `AppError::Config` if the auth config is missing.
///
/// The `implicit_hasher` allow is needed because the function passes
/// `query_params` (a `&HashMap<String, String>`) to `validate::authenticate`,
/// which invokes the default `DefaultHasher` for lookups. An explicit
/// `RandomState` type parameter would be verbose without practical benefit.
#[allow(clippy::implicit_hasher)] // passes &HashMap to validator which uses .get()
pub async fn extract_auth_info(
    state: &AppState,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<AuthInfo, AppError> {
    if endpoint.auth == "none" {
        return Ok(AuthInfo::default());
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

/// Resolve the storage store and root path for a static files endpoint.
///
/// Returns the store (Arc<dyn Storage>) and its root path (`PathBuf`).
/// For native stores, resolves the actual filesystem path.
/// For cloud stores, returns the store and a conceptual root.
fn resolve_store(
    state: &AppState,
    static_config: &StaticFilesConfig,
) -> Result<(Arc<dyn Storage>, PathBuf), AppError> {
    let store_name = &static_config.storage;
    let storage = state
        .get_store(store_name)
        .ok_or_else(|| AppError::Internal(format!("Store '{store_name}' not found")))?;

    let root = if let Some(path) = storage.root_path() {
        path
    } else {
        // For cloud stores, use the store name as a conceptual root
        PathBuf::from(store_name)
    };

    Ok((storage, root))
}

/// Handle a static file endpoint - serves files, uploads, or deletes based on method.
///
/// # Errors
///
/// Returns `AppError::Internal` if the static config is missing or the root
/// directory does not exist. Returns `AppError::NotFound` if the requested
/// file cannot be found. Returns `AppError::Forbidden` on path traversal attempts.
///
/// The `implicit_hasher` allow is needed because the function receives a
/// `Query<HashMap<String, String>>` parameter. While axum's Query extractor
/// handles parsing, the parameter type itself triggers the lint since it
/// is later passed to internal functions that use `HashMap` lookups.
#[allow(clippy::implicit_hasher)] // axum's Query<HashMap<String, String>> triggers clippy lint
pub async fn handle_static_files(
    state: State<AppState>,
    method: Method,
    uri: Uri,
    endpoint: EndpointConfig,
    headers: axum::http::HeaderMap,
    query: Option<Query<HashMap<String, String>>>,
) -> Result<Response, AppError> {
    let static_config = endpoint
        .static_files
        .as_ref()
        .ok_or_else(|| AppError::Internal("Static file configuration missing".to_string()))?;

    let (storage, root) = resolve_store(&state, static_config)?;

    if !storage.exists(&root).await {
        return Err(AppError::Internal(format!(
            "Static file storage '{}' does not exist",
            static_config.storage
        )));
    }

    let request_path = uri.path();
    let relative_str = extract_relative_path(request_path, &endpoint.path);
    let relative = percent_encoding::percent_decode_str(&relative_str)
        .decode_utf8()
        .map_err(|_| AppError::BadRequest("Invalid UTF-8 in path".to_string()))?
        .into_owned();

    if relative.split('/').any(|seg| seg == ".." || seg == ".") {
        return Err(AppError::Forbidden("Path traversal denied".to_string()));
    }

    match method {
        Method::HEAD => {
            // For HEAD, call GET handler then strip body
            let resp = crate::handlers::static_files::serving::handle_static_get(
                state,
                StaticGetContext {
                    config: static_config,
                    relative: &relative,
                    root: &root,
                    request_path,
                    query: query.as_ref(),
                    headers: &headers,
                    storage,
                },
            )
            .await?;
            let (_, body) = resp.into_parts();
            let response = Response::new(body);
            // Copy headers from the original response
            Ok(response)
        }
        Method::GET => {
            crate::handlers::static_files::serving::handle_static_get(
                state,
                StaticGetContext {
                    config: static_config,
                    relative: &relative,
                    root: &root,
                    request_path,
                    query: query.as_ref(),
                    headers: &headers,
                    storage,
                },
            )
            .await
        }
        Method::DELETE => {
            handle_file_delete(
                storage.as_ref(),
                state,
                &endpoint,
                static_config,
                &relative,
                &root,
                &headers,
            )
            .await
        }
        Method::OPTIONS => {
            let mut response = (StatusCode::OK).into_response();
            if let Some(cors) = endpoint.cors.as_ref() {
                // Extract the Origin header from the request if present.
                let origin = headers
                    .get(header::ORIGIN)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| HeaderValue::from_str(s).ok());
                cors::apply_cors_headers(&mut response, cors, origin.as_ref());
            }
            Ok(response)
        }
        _ => Err(AppError::MethodNotAllowed(format!(
            "Method {method} not allowed for this endpoint"
        ))),
    }
}

pub async fn handle_file_upload_route(
    multipart: axum::extract::Multipart,
    state: State<AppState>,
    uri: Uri,
    method: Method,
    endpoint: EndpointConfig,
    headers: axum::http::HeaderMap,
) -> Result<Response, AppError> {
    match method {
        Method::POST | Method::PUT | Method::PATCH => {
            let static_config = endpoint.static_files.as_ref().ok_or_else(|| {
                AppError::Internal("Static file configuration missing".to_string())
            })?;

            let (storage, root) = resolve_store(&state, static_config)?;

            if !storage.exists(&root).await {
                return Err(AppError::Internal(format!(
                    "Static file storage '{}' does not exist",
                    static_config.storage
                )));
            }

            let request_path = uri.path();
            let relative_str = extract_relative_path(request_path, &endpoint.path);
            let relative = percent_encoding::percent_decode_str(&relative_str)
                .decode_utf8()
                .map_err(|_| AppError::BadRequest("Invalid UTF-8 in path".to_string()))?
                .into_owned();

            if relative.split('/').any(|seg| seg == ".." || seg == ".") {
                return Err(AppError::Forbidden("Path traversal denied".to_string()));
            }

            crate::handlers::static_files::upload::handle_file_upload(
                multipart,
                state,
                &endpoint,
                static_config,
                &relative,
                &root,
                &headers,
                storage,
            )
            .await
        }
        _ => Err(AppError::MethodNotAllowed(format!(
            "Method {method} not allowed for this endpoint"
        ))),
    }
}

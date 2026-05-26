use axum::extract::Query;
use axum::extract::State;
use axum::http::Uri;
use axum::http::{Method, StatusCode, header};
use axum::response::IntoResponse;
use axum::response::Response;
use http::HeaderValue;
use std::collections::HashMap;
use std::path::Path;

use crate::config::types::EndpointConfig;
use crate::config::types::StaticFilesConfig;
use crate::error::AppError;
use crate::handlers::static_files::delete::handle_file_delete;
use crate::middleware::auth::extractor::AuthInfo;
use crate::middleware::cors;
use crate::server::state::AppState;
use crate::storage::Storage;

/// Context for handling static file GET requests.
pub struct StaticGetContext<'a> {
    pub endpoint: &'a EndpointConfig,
    pub config: &'a StaticFilesConfig,
    pub relative: &'a str,
    pub root: &'a Path,
    pub request_path: &'a str,
    pub query: Option<&'a Query<HashMap<String, String>>>,
    pub headers: &'a axum::http::HeaderMap,
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
#[allow(clippy::implicit_hasher)]
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
    crate::middleware::auth::validate::authenticate(
        &endpoint.auth,
        &auth_config,
        headers,
        query_params,
    )
    .await
}

/// Check user role against required role for upload.
pub fn check_upload_role(
    auth_info: &AuthInfo,
    upload_config: &crate::config::types::UploadConfig,
) -> Result<(), AppError> {
    if let Some(required_role) = &upload_config.required_role {
        match &auth_info.role {
            Some(role) if role == required_role => Ok(()),
            _ => Err(AppError::Forbidden(format!(
                "Role '{}' required to upload files",
                required_role
            ))),
        }
    } else {
        match &auth_info.role {
            Some(_) => Ok(()),
            None => Err(AppError::Forbidden(
                "Authentication required to upload files".to_string(),
            )),
        }
    }
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
/// is later passed to internal functions that use HashMap lookups.
#[allow(clippy::implicit_hasher)]
pub async fn handle_static_files(
    state: State<AppState>,
    method: Method,
    uri: Uri,
    endpoint: EndpointConfig,
    headers: axum::http::HeaderMap,
    query: Option<Query<HashMap<String, String>>>,
) -> Result<Response, AppError> {
    let storage = state.storage;
    let static_config = endpoint
        .static_files
        .as_ref()
        .ok_or_else(|| AppError::Internal("Static file configuration missing".to_string()))?;

    let root = Path::new(&static_config.root);
    if !storage.exists(root).await {
        return Err(AppError::Internal(format!(
            "Static file root '{}' does not exist",
            static_config.root
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
        Method::GET => {
            crate::handlers::static_files::serving::handle_static_get(
                state,
                StaticGetContext {
                    endpoint: &endpoint,
                    config: static_config,
                    relative: &relative,
                    root,
                    request_path,
                    query: query.as_ref(),
                    headers: &headers,
                },
            )
            .await
        }
        Method::DELETE => {
            handle_file_delete(
                &storage,
                state,
                &endpoint,
                static_config,
                &relative,
                root,
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
            "Method {} not allowed for this endpoint",
            method
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

            let root = Path::new(&static_config.root);
            if !root.exists() {
                return Err(AppError::Internal(format!(
                    "Static file root '{}' does not exist",
                    static_config.root
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
                root,
                &headers,
            )
            .await
        }
        _ => Err(AppError::MethodNotAllowed(format!(
            "Method {} not allowed for this endpoint",
            method
        ))),
    }
}

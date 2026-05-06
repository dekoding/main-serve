use std::collections::HashMap;
use std::path::Path;
use axum::extract::Query;
use axum::extract::State;
use axum::http::Method;
use axum::http::StatusCode;
use axum::http::Uri;
use axum::response::IntoResponse;
use axum::response::Response;

use crate::auth::middleware::AuthInfo;
use crate::config::types::EndpointConfig;
use crate::config::types::StaticFilesConfig;
use crate::error::AppError;
use crate::handlers::static_files::upload::handle_file_upload;
use crate::handlers::static_files::delete::handle_file_delete;
use crate::middleware::cors;
use crate::server::state::AppState;

/// Context for handling static file GET requests.
pub struct StaticGetContext<'a> {
    pub endpoint: &'a EndpointConfig,
    pub config: &'a StaticFilesConfig,
    pub relative: &'a str,
    pub root: &'a Path,
    pub request_path: &'a str,
    pub query: &'a Query<Option<HashMap<String, String>>>,
    pub headers: &'a axum::http::HeaderMap,
}

/// Extract authentication info from request.
pub async fn extract_auth_info(
    state: &AppState,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<AuthInfo, AppError> {
    if endpoint.auth == "none" {
        return Ok(AuthInfo::default());
    }
    crate::auth::middleware::authenticate(
        &endpoint.auth,
        &state.config.read().await.auth,
        headers,
        query_params,
    )
    .await
}

/// Check user role against required role for upload.
pub fn check_upload_role(auth_info: &AuthInfo, upload_config: &crate::config::types::UploadConfig) -> Result<(), AppError> {
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
pub async fn handle_static_files(
    state: State<AppState>,
    method: Method,
    uri: Uri,
    endpoint: EndpointConfig,
    headers: axum::http::HeaderMap,
    query: Query<Option<HashMap<String, String>>>,
) -> Result<Response, AppError> {
    let static_config = endpoint
        .static_files
        .as_ref()
        .ok_or_else(|| AppError::Internal("Static file configuration missing".to_string()))?;

    let root = Path::new(&static_config.root);
    if !root.exists() {
        return Err(AppError::Internal(format!(
            "Static file root '{}' does not exist",
            static_config.root
        )));
    }

    let request_path = uri.path();
    let ep_path = endpoint
        .path
        .trim_end_matches("/*")
        .trim_end_matches("{*rest}");
    let relative = request_path
        .strip_prefix(ep_path)
        .unwrap_or(request_path)
        .trim_start_matches('/');

    let relative = percent_encoding::percent_decode_str(relative)
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
                    query: &query,
                    headers: &headers,
                },
            )
            .await
        }
        Method::POST | Method::PUT | Method::PATCH => {
            handle_file_upload(state, &endpoint, static_config, &relative, root, &headers).await
        }
        Method::DELETE => {
            handle_file_delete(state, &endpoint, static_config, &relative, root, &headers).await
        }
        Method::OPTIONS => {
            let mut response = (StatusCode::OK).into_response();
            if let Some(cors) = endpoint.cors.as_ref() {
                cors::apply_cors_headers(&mut response, cors);
            }
            Ok(response)
        }
        _ => Err(AppError::MethodNotAllowed(format!(
            "Method {} not allowed for this endpoint",
            method
        ))),
    }
}
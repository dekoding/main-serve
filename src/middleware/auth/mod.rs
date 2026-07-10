/// Auth middleware module: validation, extractors, handlers, and layer.
pub mod extractor;
/// handler
pub mod handler;
/// validate
pub mod validate;
/// validators
pub mod validators;

use axum::middleware::Next;
use axum::{body::Body, http::Request, response::Response};

use crate::error::AppError;
use crate::server::AppState;

/// Routes that should skip authentication.
const SKIP_ROUTES: &[&str] = &[
    "/_main-serve/health",
    "/_main-serve/reload",
    "/_main-serve/oauth2/authorize",
    "/_main-serve/oauth2/callback",
];

/// Auth middleware that checks authentication for each request.
///
/// This middleware runs before handlers and:
/// 1. Skips auth for routes in `SKIP_ROUTES`
/// 2. Extracts endpoint config from router state
/// 3. Validates credentials based on endpoint auth config
/// 4. Checks role authorization
/// 5. Inserts auth info into request extensions for handlers
pub async fn auth_middleware(
    state: axum::extract::State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let path = req.uri().path().to_string();

    if SKIP_ROUTES.iter().any(|skip| path.starts_with(skip)) {
        return Ok(next.run(req).await);
    }

    let endpoint_config = {
        state
            .0
            .get_endpoint_config_for_method(req.uri().path(), req.method())
            .await
    };

    let auth_type = endpoint_config
        .as_ref()
        .map_or_else(|| "none".to_string(), |e| e.auth.clone());

    let auth_config = if auth_type == "none" {
        None
    } else {
        Some(state.0.config.read().await.auth.clone())
    };

    let headers = req.headers().clone();
    let query_params: std::collections::HashMap<String, String> = req
        .uri()
        .query()
        .map(|q| {
            q.split('&')
                .filter_map(|p| {
                    p.split_once('=')
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();

    let required_roles = endpoint_config
        .as_ref()
        .and_then(|e| {
            e.methods
                .iter()
                .find(|m| m.matches(req.method()))
                .map(|m| e.roles.clone_for_method(*m))
        })
        .unwrap_or_default();

    if auth_type == "none" {
        return Ok(next.run(req).await);
    }

    let auth_config = auth_config
        .ok_or_else(|| AppError::Config("Auth required but no auth config provided".to_string()))?;

    let revocation_store = state.0.revocation_store.get();
    let auth_info = crate::middleware::auth::validate::authenticate(
        &auth_type,
        &auth_config,
        &headers,
        &query_params,
        revocation_store,
    )
    .await?;

    let role_inheritance = state.0.role_inheritance.read().await;
    crate::middleware::auth::validate::check_roles(&auth_info, &required_roles, &role_inheritance)?;

    req.extensions_mut().insert(auth_info);

    Ok(next.run(req).await)
}

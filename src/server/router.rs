use futures_util::future::FutureExt;
use std::collections::HashMap;
use tower_http::compression::CompressionLayer;

use axum::Router;
use axum::body::Body;
use axum::debug_handler;
use axum::extract::Multipart;
use axum::extract::{MatchedPath, Path, Query, State};
use axum::http::{HeaderMap, Method as HttpMethod, Uri};
use axum::response::{IntoResponse, Response};
use http::Extensions;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};

use super::prefix_match::find_prefix_match;
use super::reload::{handle_health, handle_reload};
use super::state::AppState;
use crate::config::AppConfig;
use crate::config::types::{EndpointAction, EndpointConfig, HttpMethod as ConfigHttpMethod};
use crate::error::AppError;
use crate::handlers::crud::handle_crud;
use crate::handlers::custom_response::handle_custom_response;
use crate::handlers::proxy::handle_proxy;
use crate::handlers::static_files::routing::{handle_file_upload_route, handle_static_files};
use crate::middleware::auth::extractor::{AuthInfo, RequestContext};
use crate::middleware::auth::{
    auth_middleware,
    handler::{handle_oauth2_authorize, handle_oauth2_callback},
};
use crate::middleware::body_limit::body_limit_middleware;
use crate::middleware::cors::build_cors_layer;
use crate::middleware::logging::{body_logging_middleware, build_trace_layer};
use crate::middleware::rate_limit::rate_limit_middleware;

pub async fn build_router(config: &AppConfig, state: AppState) -> Router {
    let mut endpoint_configs = std::collections::HashMap::new();
    for endpoint in &config.endpoints {
        let mut path = endpoint.path.replace(":id", "{id}").replace('*', "{*rest}");
        let static_catch_all_added =
            endpoint.action == EndpointAction::Static && !path.contains("{*rest}");
        if static_catch_all_added {
            let trimmed = path.trim_end_matches('/');
            path = format!("{trimmed}/{{*rest}}");
        }

        // Store endpoint config with path#method key for each method
        // This allows different auth settings per method for the same path
        for method in &endpoint.methods {
            let method_str = method.as_str();
            let method_path = format!("{}#{}", path, method_str);
            endpoint_configs.insert(method_path, endpoint.clone());
        }

        if endpoint.action == EndpointAction::Static {
            let bare = path.trim_end_matches("{*rest}").trim_end_matches('/');
            if !bare.is_empty() {
                endpoint_configs.insert(bare.to_string(), endpoint.clone());
                endpoint_configs.insert(format!("{bare}/"), endpoint.clone());
            } else {
                endpoint_configs.insert("/".to_string(), endpoint.clone());
            }
        }
    }

    let mut configs_write = state.endpoint_configs.write().await;
    *configs_write = endpoint_configs;
    drop(configs_write);

    let mut router = Router::new()
        .route("/_main-serve/health", axum::routing::get(handle_health))
        .route("/_main-serve/reload", axum::routing::post(handle_reload));

    if config
        .auth
        .oauth2
        .as_ref()
        .is_some_and(|o| !o.authorization_url.is_empty())
    {
        router = router
            .route(
                "/_main-serve/oauth2/authorize",
                axum::routing::get(handle_oauth2_authorize),
            )
            .route(
                "/_main-serve/oauth2/callback",
                axum::routing::get(handle_oauth2_callback),
            );
    }

    for endpoint in &config.endpoints {
        let path_for_routes = endpoint.path.replace(":id", "{id}").replace('*', "{*rest}");
        let static_catch_all_added =
            endpoint.action == EndpointAction::Static && !path_for_routes.contains("{*rest}");
        let path = if static_catch_all_added {
            let trimmed = path_for_routes.trim_end_matches('/');
            format!("{trimmed}/{{*rest}}")
        } else {
            path_for_routes
        };

        // Always provide a CORS config for per-endpoint routing:
        // - Use endpoint's own CORS if present
        // - Otherwise, fall back to global CORS config
        let endpoint_cors = endpoint.cors.as_ref().unwrap_or(&config.cors);

        // Combine all methods into a single route to enable CORS preflight support
        // and avoid CORS conflicts from multiple route_layer calls
        let mut combined_router = Router::new();
        for method in &endpoint.methods {
            combined_router = add_endpoint_route(
                combined_router,
                &path,
                *method,
                endpoint,
                Some(endpoint_cors),
            );
        }

        // Apply CORS layer to the combined router for this endpoint
        let combined_router = combined_router.layer(build_cors_layer(endpoint_cors));

        router = router.merge(combined_router);

        if endpoint.action == EndpointAction::Static && path.ends_with("{*rest}") {
            let bare = path.trim_end_matches("{*rest}").trim_end_matches('/');
            if bare.is_empty() {
                let mut combined_router = Router::new();
                for method in &endpoint.methods {
                    combined_router = add_endpoint_route(
                        combined_router,
                        "/",
                        *method,
                        endpoint,
                        Some(endpoint_cors),
                    );
                }
                if !endpoint.methods.contains(&ConfigHttpMethod::Options) {
                    combined_router = add_endpoint_route(
                        combined_router,
                        "/",
                        ConfigHttpMethod::Options,
                        endpoint,
                        Some(endpoint_cors),
                    );
                }
                let combined_router = combined_router.layer(build_cors_layer(endpoint_cors));
                router = router.merge(combined_router);

                let with_slash = format!("{bare}/");
                if !bare.is_empty() {
                    let mut combined_router = Router::new();
                    for method in &endpoint.methods {
                        combined_router = add_endpoint_route(
                            combined_router,
                            &with_slash,
                            *method,
                            endpoint,
                            Some(endpoint_cors),
                        );
                    }
                    if !endpoint.methods.contains(&ConfigHttpMethod::Options) {
                        combined_router = add_endpoint_route(
                            combined_router,
                            &with_slash,
                            ConfigHttpMethod::Options,
                            endpoint,
                            Some(endpoint_cors),
                        );
                    }
                    let combined_router = combined_router.layer(build_cors_layer(endpoint_cors));
                    router = router.merge(combined_router);
                }
            } else {
                let mut combined_router = Router::new();
                for method in &endpoint.methods {
                    combined_router = add_endpoint_route(
                        combined_router,
                        bare,
                        *method,
                        endpoint,
                        Some(endpoint_cors),
                    );
                }
                if !endpoint.methods.contains(&ConfigHttpMethod::Options) {
                    combined_router = add_endpoint_route(
                        combined_router,
                        bare,
                        ConfigHttpMethod::Options,
                        endpoint,
                        Some(endpoint_cors),
                    );
                }
                let combined_router = combined_router.layer(build_cors_layer(endpoint_cors));
                router = router.merge(combined_router);

                let with_slash = format!("{bare}/");
                let mut combined_router = Router::new();
                for method in &endpoint.methods {
                    combined_router = add_endpoint_route(
                        combined_router,
                        &with_slash,
                        *method,
                        endpoint,
                        Some(endpoint_cors),
                    );
                }
                if !endpoint.methods.contains(&ConfigHttpMethod::Options) {
                    combined_router = add_endpoint_route(
                        combined_router,
                        &with_slash,
                        ConfigHttpMethod::Options,
                        endpoint,
                        Some(endpoint_cors),
                    );
                }
                let combined_router = combined_router.layer(build_cors_layer(endpoint_cors));
                router = router.merge(combined_router);
            }
        }
    }

    let router = router.with_state(state.clone());

    // Body limit must run before body logging so it can reconstruct the body first.
    let router = router.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        body_limit_middleware,
    ));

    let router = if config.logging.log_request_body || config.logging.log_response_body {
        router.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            body_logging_middleware,
        ))
    } else {
        router
    };

    // Apply global CORS as a base layer.

    // Auth middleware must run before body logging so auth info is available in
    // request extensions for logging. Auth info is also needed for token-based
    // rate limiting which runs after this.
    let router = router.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        auth_middleware,
    ));

    // Rate limiting must run after auth so it can use auth info (token) for
    // token-based rate limiting keys.
    let router = router.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        rate_limit_middleware,
    ));

    router
        .layer(CompressionLayer::new())
        .layer(build_trace_layer())
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
}

/// Look up an endpoint config by prefix-matching the path.
/// Used by route handlers when method-specific lookup fails.
async fn lookup_endpoint_by_prefix(state: &State<AppState>, path: &str) -> Option<EndpointConfig> {
    let configs = state.endpoint_configs.read().await;
    find_prefix_match(&configs, path, |_| true)
}

fn add_endpoint_route(
    mut app: Router<AppState>,
    path: &str,
    method: ConfigHttpMethod,
    endpoint: &EndpointConfig,
    cors: Option<&crate::config::types::CorsConfig>,
) -> Router<AppState> {
    match endpoint.action {
        EndpointAction::CustomResponse => {
            let handler = handle_custom_response_route;
            let method_router = match method {
                ConfigHttpMethod::Get => axum::routing::get(handler),
                ConfigHttpMethod::Post => axum::routing::post(handler),
                ConfigHttpMethod::Put => axum::routing::put(handler),
                ConfigHttpMethod::Patch => axum::routing::patch(handler),
                ConfigHttpMethod::Delete => axum::routing::delete(handler),
                ConfigHttpMethod::Head => axum::routing::head(handler),
                ConfigHttpMethod::Options => axum::routing::options(handler),
            };

            if let Some(cors_config) = cors {
                app = app
                    .route(path, method_router)
                    .route_layer(build_cors_layer(cors_config));
            } else {
                app = app.route(path, method_router);
            }
        }

        EndpointAction::Crud => {
            let handler = handle_crud_route;
            let method_router = match method {
                ConfigHttpMethod::Get => axum::routing::get(handler),
                ConfigHttpMethod::Post => axum::routing::post(handler),
                ConfigHttpMethod::Put => axum::routing::put(handler),
                ConfigHttpMethod::Patch => axum::routing::patch(handler),
                ConfigHttpMethod::Delete => axum::routing::delete(handler),
                ConfigHttpMethod::Head => axum::routing::head(handler),
                ConfigHttpMethod::Options => axum::routing::options(handler),
            };

            if let Some(cors_config) = cors {
                app = app
                    .route(path, method_router)
                    .route_layer(build_cors_layer(cors_config));
            } else {
                app = app.route(path, method_router);
            }
        }

        EndpointAction::Proxy => {
            let handler = handle_proxy_route;
            let method_router = match method {
                ConfigHttpMethod::Get => axum::routing::get(handler),
                ConfigHttpMethod::Post => axum::routing::post(handler),
                ConfigHttpMethod::Put => axum::routing::put(handler),
                ConfigHttpMethod::Patch => axum::routing::patch(handler),
                ConfigHttpMethod::Delete => axum::routing::delete(handler),
                ConfigHttpMethod::Head => axum::routing::head(handler),
                ConfigHttpMethod::Options => axum::routing::options(handler),
            };

            if let Some(cors_config) = cors {
                app = app
                    .route(path, method_router)
                    .route_layer(build_cors_layer(cors_config));
            } else {
                app = app.route(path, method_router);
            }
        }

        EndpointAction::Static => {
            let has_upload = endpoint
                .static_files
                .as_ref()
                .and_then(|sf| sf.upload.as_ref())
                .is_some_and(|u| u.enabled);

            if has_upload {
                let handler = handle_upload_route;
                let method_router = match method {
                    ConfigHttpMethod::Post => axum::routing::post(handler),
                    ConfigHttpMethod::Put => axum::routing::put(handler),
                    ConfigHttpMethod::Patch => axum::routing::patch(handler),
                    ConfigHttpMethod::Delete => axum::routing::delete(handle_static_files_route),
                    ConfigHttpMethod::Head => axum::routing::head(handle_static_files_route),
                    ConfigHttpMethod::Options => axum::routing::options(handle_static_files_route),
                    ConfigHttpMethod::Get => axum::routing::get(handle_static_files_route),
                };

                if let Some(cors_config) = cors {
                    app = app
                        .route(path, method_router)
                        .route_layer(build_cors_layer(cors_config));
                } else {
                    app = app.route(path, method_router);
                }
            } else {
                let handler = handle_static_files_route;
                let method_router = match method {
                    ConfigHttpMethod::Get => axum::routing::get(handler),
                    ConfigHttpMethod::Post => axum::routing::post(handler),
                    ConfigHttpMethod::Put => axum::routing::put(handler),
                    ConfigHttpMethod::Patch => axum::routing::patch(handler),
                    ConfigHttpMethod::Delete => axum::routing::delete(handler),
                    ConfigHttpMethod::Head => axum::routing::head(handler),
                    ConfigHttpMethod::Options => axum::routing::options(handler),
                };

                if let Some(cors_config) = cors {
                    app = app
                        .route(path, method_router)
                        .route_layer(build_cors_layer(cors_config));
                } else {
                    app = app.route(path, method_router);
                }
            }
        }
    }
    app
}

#[debug_handler]
async fn handle_custom_response_route(
    state: State<AppState>,
    method: axum::http::Method,
    matched_path: MatchedPath,
) -> Result<Response, AppError> {
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config_for_method(path_str, &method)
        .await
        .or_else(|| {
            lookup_endpoint_by_prefix(&state, path_str)
                .now_or_never()
                .flatten()
        })
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    Ok(handle_custom_response(state, endpoint)
        .await
        .into_response())
}

#[debug_handler]
#[allow(clippy::too_many_arguments)]
async fn handle_crud_route(
    state: State<AppState>,
    matched_path: MatchedPath,
    extensions: Extensions,
    method: HttpMethod,
    headers: HeaderMap,
    path_params: Option<Path<HashMap<String, String>>>,
    query: Query<HashMap<String, String>>,
    body: Body,
) -> Result<Response, AppError> {
    let query_map = query.0.clone();
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config_for_method(path_str, &method)
        .await
        .or_else(|| {
            lookup_endpoint_by_prefix(&state, path_str)
                .now_or_never()
                .flatten()
        })
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    let json_body: Option<axum::Json<serde_json::Value>> = if headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|t| t.contains("application/json"))
        .unwrap_or(false)
    {
        let bytes = axum::body::to_bytes(body, usize::MAX)
            .await
            .map_err(|e| AppError::Body(e.to_string()))?;
        serde_json::from_slice(&bytes)
            .map_err(|e| AppError::ParseError(e.to_string()))
            .ok()
            .map(axum::Json)
    } else {
        None
    };

    let auth_info = extensions.get::<AuthInfo>().cloned().unwrap_or_default();
    let context = RequestContext {
        user_id: auth_info.subject.clone().into(),
        user_role: auth_info.role,
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect(),
        method: method.to_string(),
        path: path_str.to_string(),
        query_params: query_map.clone(),
    };
    let response = handle_crud(
        state,
        method,
        path_params,
        Query(query_map),
        json_body,
        endpoint,
        context,
    )
    .await?;
    Ok(response.into_response())
}

#[debug_handler]
async fn handle_proxy_route(
    state: State<AppState>,
    matched_path: MatchedPath,
    method: HttpMethod,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, AppError> {
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config_for_method(path_str, &method)
        .await
        .or_else(|| {
            lookup_endpoint_by_prefix(&state, path_str)
                .now_or_never()
                .flatten()
        })
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    handle_proxy(state, method, uri, headers, body, endpoint).await
}

#[debug_handler]
async fn handle_upload_route(
    state: State<AppState>,
    matched_path: MatchedPath,
    method: axum::http::Method,
    uri: Uri,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config_for_method(path_str, &method)
        .await
        .or_else(|| {
            lookup_endpoint_by_prefix(&state, path_str)
                .now_or_never()
                .flatten()
        })
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    handle_file_upload_route(multipart, state, uri, method, endpoint, headers).await
}

#[debug_handler]
async fn handle_static_files_route(
    state: State<AppState>,
    matched_path: MatchedPath,
    method: axum::http::Method,
    uri: Uri,
    query: Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config_for_method(path_str, &method)
        .await
        .or_else(|| {
            lookup_endpoint_by_prefix(&state, path_str)
                .now_or_never()
                .flatten()
        })
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    let response = handle_static_files(state, method, uri, endpoint, headers, Some(query)).await?;
    Ok(response)
}

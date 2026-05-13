use futures_util::future::FutureExt;
use std::collections::HashMap;
use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::debug_handler;
use axum::extract::Multipart;
use axum::extract::{Extension, MatchedPath, Path, Query, State};
use axum::http::{HeaderMap, Method as HttpMethod, Uri};
use axum::response::{IntoResponse, Response};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};

use super::reload::{handle_health, handle_reload};
use super::state::AppState;
use crate::auth::middleware::{authenticate, check_roles};
use crate::auth::oauth2::{handle_oauth2_authorize, handle_oauth2_callback};
use crate::config::AppConfig;
use crate::config::types::{EndpointAction, EndpointConfig, HttpMethod as ConfigHttpMethod};
use crate::error::AppError;
use crate::handlers::crud::handle_crud;
use crate::handlers::custom_response::handle_custom_response;
use crate::handlers::proxy::handle_proxy;
use crate::handlers::static_files::routing::{handle_file_upload_route, handle_static_files};
use crate::middleware::body_limit::body_limit_middleware;
use crate::middleware::compression::build_compression_layer;
use crate::middleware::cors::build_cors_layer;
use crate::middleware::logging::{body_logging_middleware, build_trace_layer};

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
        endpoint_configs.insert(path.clone(), endpoint.clone());
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
        
        for method in &endpoint.methods {
            router = add_endpoint_route(router, &path, *method, endpoint, Some(endpoint_cors));
        }

        if endpoint.action == EndpointAction::Static && path.ends_with("{*rest}") {
            let bare = path.trim_end_matches("{*rest}").trim_end_matches('/');
            if bare.is_empty() {
                for method in &endpoint.methods {
                    router = add_endpoint_route(router, "/", *method, endpoint, Some(endpoint_cors));
                }
            } else {
                for method in &endpoint.methods {
                    router = add_endpoint_route(router, bare, *method, endpoint, Some(endpoint_cors));
                    let with_slash = format!("{bare}/");
                    router = add_endpoint_route(router, &with_slash, *method, endpoint, Some(endpoint_cors));
                }
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

    // Per-endpoint CORS is now applied in add_endpoint_route for all endpoints.
    // No global CORS layer needed.

    router
        .layer(build_compression_layer())
        .layer(build_trace_layer())
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
}

fn extract_addr(ext: Option<Extension<SocketAddr>>) -> Option<SocketAddr> {
    ext.map(|Extension(a)| a)
}

fn find_prefix_match<'a>(
    state: &'a State<AppState>,
    path: &'a str,
) -> impl std::future::Future<Output = Option<EndpointConfig>> + 'a {
    async move {
        let configs = state.endpoint_configs.read().await;
        let mut current = path;
        while !current.is_empty() {
            if let Some(endpoint) = configs.get(current) {
                return Some(endpoint.clone());
            }
            if let Some(pos) = current.rfind('/') {
                current = &current[..pos];
            } else {
                break;
            }
        }
        None
    }
}

fn add_endpoint_route(
    mut app: Router<AppState>,
    path: &str,
    method: ConfigHttpMethod,
    _endpoint: &EndpointConfig,
    cors: Option<&crate::config::types::CorsConfig>,
) -> Router<AppState> {
    match _endpoint.action {
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
                app = app.route(path, method_router).route_layer(build_cors_layer(cors_config));
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
                app = app.route(path, method_router).route_layer(build_cors_layer(cors_config));
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
                app = app.route(path, method_router).route_layer(build_cors_layer(cors_config));
            } else {
                app = app.route(path, method_router);
            }
        }

        EndpointAction::Static => {
            let has_upload = _endpoint
                .static_files
                .as_ref()
                .and_then(|sf| sf.upload.as_ref())
                .is_some_and(|u| u.enabled);

            if has_upload {
                let handler = handle_upload_route;
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
                    app = app.route(path, method_router).route_layer(build_cors_layer(cors_config));
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
                    app = app.route(path, method_router).route_layer(build_cors_layer(cors_config));
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
    matched_path: MatchedPath,
    remote_addr: Option<Extension<SocketAddr>>,
    headers: HeaderMap,
    query: Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let query_map = query.0.clone();
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config(path_str)
        .await
        .or_else(|| find_prefix_match(&state, path_str).now_or_never().flatten())
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    let state_clone = state.clone();
    run_pre_checks(
        state_clone,
        &headers,
        &query_map,
        &endpoint,
        extract_addr(remote_addr),
    )
    .await?;
    Ok(handle_custom_response(state, endpoint)
        .await
        .into_response())
}

#[debug_handler]
async fn handle_crud_route(
    state: State<AppState>,
    matched_path: MatchedPath,
    remote_addr: Option<Extension<SocketAddr>>,
    method: HttpMethod,
    headers: HeaderMap,
    path_params: Option<Path<HashMap<String, String>>>,
    query: Query<HashMap<String, String>>,
    body: Body,
) -> Result<Response, AppError> {
    let query_map = query.0.clone();
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config(path_str)
        .await
        .or_else(|| find_prefix_match(&state, path_str).now_or_never().flatten())
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

    let state_clone = state.clone();
    let auth_info = run_pre_checks(
        state_clone,
        &headers,
        &query_map,
        &endpoint,
        extract_addr(remote_addr),
    )
    .await?;
    let context = crate::context::RequestContext {
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
    remote_addr: Option<Extension<SocketAddr>>,
    method: HttpMethod,
    uri: Uri,
    query: Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, AppError> {
    let query_map = query.0.clone();
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config(path_str)
        .await
        .or_else(|| find_prefix_match(&state, path_str).now_or_never().flatten())
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    let state_clone = state.clone();
    run_pre_checks(
        state_clone,
        &headers,
        &query_map,
        &endpoint,
        extract_addr(remote_addr),
    )
    .await?;
    handle_proxy(state, method, uri, headers, body, endpoint).await
}

#[debug_handler]
async fn handle_upload_route(
    state: State<AppState>,
    matched_path: MatchedPath,
    remote_addr: Option<Extension<SocketAddr>>,
    method: axum::http::Method,
    uri: Uri,
    query: Query<HashMap<String, String>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let query_map = query.0.clone();
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config(path_str)
        .await
        .or_else(|| find_prefix_match(&state, path_str).now_or_never().flatten())
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    let state_clone = state.clone();
    run_pre_checks(
        state_clone,
        &headers,
        &query_map,
        &endpoint,
        extract_addr(remote_addr),
    )
    .await?;
    handle_file_upload_route(multipart, state, uri, method, endpoint, headers).await
}

#[debug_handler]
async fn handle_static_files_route(
    state: State<AppState>,
    matched_path: MatchedPath,
    remote_addr: Option<Extension<SocketAddr>>,
    method: axum::http::Method,
    uri: Uri,
    query: Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let query_map = query.0.clone();
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config(path_str)
        .await
        .or_else(|| find_prefix_match(&state, path_str).now_or_never().flatten())
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    let state_clone = state.clone();
    run_pre_checks(
        state_clone,
        &headers,
        &query_map,
        &endpoint,
        extract_addr(remote_addr),
    )
    .await?;
    let response = handle_static_files(
        state,
        method,
        uri,
        endpoint,
        headers,
        Some(Query(query_map)),
    )
    .await?;
    Ok(response)
}

async fn run_pre_checks(
    state: axum::extract::State<AppState>,
    headers: &HeaderMap,
    query_params: &HashMap<String, String>,
    endpoint: &EndpointConfig,
    remote_addr: Option<std::net::SocketAddr>,
) -> Result<crate::auth::middleware::AuthInfo, AppError> {
    let config = state.0.config.read().await;
    let rl_config = endpoint.rate_limit.as_ref().unwrap_or(&config.rate_limit);
    state
        .rate_limiter
        .check_rate_limit(rl_config, headers, remote_addr)
        .await?;

    if endpoint.auth == "none" {
        return Ok(crate::auth::middleware::AuthInfo::default());
    }

    let auth_info = authenticate(&endpoint.auth, &config.auth, headers, query_params).await?;
    drop(config);

    check_roles(&auth_info, &endpoint.roles)?;

    Ok(auth_info)
}

// /// Route a handler to a specific HTTP method on a path.
// ///
// /// If a per-endpoint CORS config is provided, it is applied as a route-level
// /// layer by nesting the route in a sub-router, overriding the global CORS config.
// fn route_method<F>(
//     mut app: Router<AppState>,
//     path: &str,
//     method: HttpMethod,
//     handler: F,
//     cors_override: Option<&crate::config::types::CorsConfig>,
// ) -> Router<AppState>
// where
//     F: axum::handler::Handler<(), AppState> + Clone + Send + 'static,
// {
//     let method_router = match method {
//         HttpMethod::Get => axum::routing::get(handler.clone()),
//         HttpMethod::Post => axum::routing::post(handler.clone()),
//         HttpMethod::Put => axum::routing::put(handler.clone()),
//         HttpMethod::Patch => axum::routing::patch(handler.clone()),
//         HttpMethod::Delete => axum::routing::delete(handler.clone()),
//         HttpMethod::Head => axum::routing::head(handler.clone()),
//         HttpMethod::Options => axum::routing::options(handler.clone()),
//     };

//     if let Some(cors_config) = cors_override {
//         let sub = Router::<AppState>::new()
//             .route(path, method_router)
//             .layer(build_cors_layer(cors_config));
//         app.merge(sub)
//     } else {
//         app.route(path, method_router)
//     }
// }

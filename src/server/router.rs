/// Dynamic router builder - constructs an axum Router from parsed config.
///
/// This module builds a complete `axum::Router` from an `AppConfig` struct.
/// The router includes:
/// - Hard-coded system endpoints (`/_main-serve/health`, `/_main-serve/reload`)
/// - All user-defined endpoints from the YAML config
///
/// The router is rebuilt on every hot-reload and broadcast via a `watch` channel.
/// Each endpoint's auth and role requirements are enforced before the handler runs.
use std::collections::HashMap;
use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Extension, Path, Query};
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};

use super::reload::{handle_health, handle_reload};
use super::state::AppState;
use crate::auth::middleware::{authenticate, check_roles};
use crate::auth::oauth2::{handle_oauth2_authorize, handle_oauth2_callback};
use crate::config::AppConfig;
use crate::config::types::{EndpointAction, EndpointConfig, HttpMethod};
use crate::error::AppError;
use crate::handlers::crud::handle_crud;
use crate::handlers::custom_response::handle_custom_response;
use crate::handlers::proxy::handle_proxy;
use crate::handlers::static_files::handle_static_files;
use crate::middleware::compression::build_compression_layer;
use crate::middleware::cors::build_cors_layer;
use crate::middleware::logging::{body_logging_middleware, build_trace_layer};

/// Build a complete `Router` from the given config and shared state.
///
/// This is called at startup and again on every successful hot-reload.
pub fn build_router(config: &AppConfig, state: AppState) -> Router {
    let mut app = Router::new()
        // Hard-coded system endpoints - always present, not configurable.
        .route("/_main-serve/health", get(handle_health))
        .route("/_main-serve/reload", post(handle_reload));

    // Register OAuth2 code flow endpoints if the authorization URL is configured.
    if config
        .auth
        .oauth2
        .as_ref()
        .map(|o| !o.authorization_url.is_empty())
        .unwrap_or(false)
    {
        app = app
            .route(
                "/_main-serve/oauth2/authorize",
                get(handle_oauth2_authorize),
            )
            .route("/_main-serve/oauth2/callback", get(handle_oauth2_callback));
    }

    // Check if any endpoints have per-endpoint CORS overrides.
    let has_per_endpoint_cors = config.endpoints.iter().any(|ep| ep.cors.is_some());

    // Wire user-defined endpoints from config.
    for endpoint in &config.endpoints {
        let mut path = endpoint.path.replace(":id", "{id}").replace("*", "{*rest}");

        // Static endpoints need a catch-all suffix to serve files under
        // the root directory.  If the user omitted the trailing /* in the
        // config path, append /{*rest} automatically so that sub-paths
        // (e.g. /site/style.css) are routed to the handler.
        let static_catch_all_added =
            endpoint.action == EndpointAction::Static && !path.contains("{*rest}");
        if static_catch_all_added {
            let trimmed = path.trim_end_matches('/');
            path = format!("{trimmed}/{{*rest}}");
        }

        // Determine the effective CORS config for this endpoint.
        // If any endpoint uses per-endpoint CORS, we apply CORS per-route
        // (using the endpoint's override or the global config) so that
        // per-endpoint overrides aren't masked by the global CORS layer.
        let cors_ref = if has_per_endpoint_cors {
            Some(endpoint.cors.as_ref().unwrap_or(&config.cors))
        } else {
            None
        };

        for method in &endpoint.methods {
            app = add_endpoint_route(app, &path, *method, endpoint, cors_ref);
        }

        // For static endpoints, also register the bare path (without the
        // catch-all) so that e.g. GET /static serves index.html just like
        // GET /static/ does. Without this, axum's {*rest} catch-all only
        // matches when there is at least a trailing slash.
        if endpoint.action == EndpointAction::Static && path.ends_with("{*rest}") {
            let bare = path.trim_end_matches("{*rest}").trim_end_matches('/');
            if bare.is_empty() {
                // Endpoint mounted at root "/": register "/" so that
                // GET / is handled (the catch-all /{*rest} alone won't
                // match the bare root).
                for method in &endpoint.methods {
                    app = add_endpoint_route(app, "/", *method, endpoint, cors_ref);
                }
            } else {
                for method in &endpoint.methods {
                    app = add_endpoint_route(app, bare, *method, endpoint, cors_ref);
                    // Also register with trailing slash to cover /static/ explicitly,
                    // since registering /static as a separate route can prevent
                    // matchit from falling through to the catch-all for /static/.
                    let with_slash = format!("{bare}/");
                    app = add_endpoint_route(app, &with_slash, *method, endpoint, cors_ref);
                }
            }
        }
    }

    let router = app.with_state(state.clone());

    // If no endpoint uses per-endpoint CORS, apply the global CORS layer once
    // at the top level. Otherwise, CORS was already applied per-route above.
    let router = if has_per_endpoint_cors {
        router
    } else {
        router.layer(build_cors_layer(&config.cors))
    };

    // Conditionally add the body logging middleware when either flag is enabled.
    let router = if config.logging.log_request_body || config.logging.log_response_body {
        router.layer(axum::middleware::from_fn_with_state(
            state,
            body_logging_middleware,
        ))
    } else {
        router
    };

    router
        // Layers are applied bottom-up: request ID first, then tracing, compression, body limit.
        .layer(DefaultBodyLimit::max(config.server.max_body_size))
        .layer(build_compression_layer())
        .layer(build_trace_layer())
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
}

/// Extract the remote address from the request extension.
fn extract_addr(ext: Option<Extension<SocketAddr>>) -> Option<SocketAddr> {
    ext.map(|Extension(a)| a)
}

/// Add a single method handler for an endpoint to the router.
fn add_endpoint_route(
    app: Router<AppState>,
    path: &str,
    method: HttpMethod,
    endpoint: &EndpointConfig,
    cors: Option<&crate::config::types::CorsConfig>,
) -> Router<AppState> {
    let ep = endpoint.clone();
    match endpoint.action {
        EndpointAction::CustomResponse => {
            let handler = move |state: axum::extract::State<AppState>,
                                remote_addr: Option<Extension<SocketAddr>>,
                                query: Query<HashMap<String, String>>,
                                headers: HeaderMap| {
                let ep = ep.clone();
                async move {
                    run_pre_checks(&state, &headers, &query.0, &ep, extract_addr(remote_addr))
                        .await?;
                    Ok::<Response, AppError>(
                        handle_custom_response(axum::extract::State(state.0), ep)
                            .await
                            .into_response(),
                    )
                }
            };
            route_method(app, path, method, handler, cors)
        }
        EndpointAction::Crud => {
            let handler =
                move |state: axum::extract::State<AppState>,
                      method: axum::http::Method,
                      path_params: Option<Path<HashMap<String, String>>>,
                      remote_addr: Option<Extension<SocketAddr>>,
                      query: Query<HashMap<String, String>>,
                      headers: HeaderMap,
                      body: Option<axum::Json<serde_json::Value>>| {
                    let ep = ep.clone();
                    async move {
                        run_pre_checks(&state, &headers, &query.0, &ep, extract_addr(remote_addr))
                            .await?;
                        handle_crud(
                            axum::extract::State(state.0),
                            method,
                            path_params,
                            Query(query.0),
                            body,
                            ep,
                        )
                        .await
                        .map(IntoResponse::into_response)
                    }
                };
            route_method(app, path, method, handler, cors)
        }
        EndpointAction::Proxy => {
            let handler = move |state: axum::extract::State<AppState>,
                                method: axum::http::Method,
                                uri: Uri,
                                remote_addr: Option<Extension<SocketAddr>>,
                                query: Query<HashMap<String, String>>,
                                headers: HeaderMap,
                                body: Body| {
                let ep = ep.clone();
                async move {
                    run_pre_checks(&state, &headers, &query.0, &ep, extract_addr(remote_addr))
                        .await?;
                    handle_proxy(
                        axum::extract::State(state.0),
                        method,
                        uri,
                        headers,
                        body,
                        ep,
                    )
                    .await
                }
            };
            route_method(app, path, method, handler, cors)
        }
        EndpointAction::Static => {
            let handler = move |state: axum::extract::State<AppState>,
                                uri: Uri,
                                remote_addr: Option<Extension<SocketAddr>>,
                                query: Query<HashMap<String, String>>,
                                headers: HeaderMap| {
                let ep = ep.clone();
                async move {
                    run_pre_checks(&state, &headers, &query.0, &ep, extract_addr(remote_addr))
                        .await?;
                    handle_static_files(axum::extract::State(state.0), uri, ep).await
                }
            };
            route_method(app, path, method, handler, cors)
        }
    }
}

/// Run rate limiting, authentication, and role-based authorization for an endpoint.
async fn run_pre_checks(
    state: &axum::extract::State<AppState>,
    headers: &HeaderMap,
    query_params: &HashMap<String, String>,
    endpoint: &EndpointConfig,
    remote_addr: Option<std::net::SocketAddr>,
) -> Result<(), AppError> {
    // Rate limiting comes first - reject early before auth overhead.
    let config = state.config.read().await;
    let rl_config = endpoint.rate_limit.as_ref().unwrap_or(&config.rate_limit);
    state
        .rate_limiter
        .check_rate_limit(rl_config, headers, remote_addr)
        .await?;

    if endpoint.auth == "none" {
        return Ok(());
    }

    let auth_info = authenticate(&endpoint.auth, &config.auth, headers, query_params).await?;
    drop(config);

    check_roles(&auth_info, &endpoint.roles)?;

    Ok(())
}

/// Route a handler to a specific HTTP method on a path.
///
/// If a per-endpoint CORS config is provided, it is applied as a route-level
/// layer by nesting the route in a sub-router, overriding the global CORS config.
fn route_method<H, T>(
    app: Router<AppState>,
    path: &str,
    method: HttpMethod,
    handler: H,
    cors_override: Option<&crate::config::types::CorsConfig>,
) -> Router<AppState>
where
    H: axum::handler::Handler<T, AppState> + Clone,
    T: 'static,
{
    let method_router = match method {
        HttpMethod::Get => axum::routing::get(handler),
        HttpMethod::Post => axum::routing::post(handler),
        HttpMethod::Put => axum::routing::put(handler),
        HttpMethod::Patch => axum::routing::patch(handler),
        HttpMethod::Delete => axum::routing::delete(handler),
        HttpMethod::Head => axum::routing::head(handler),
        HttpMethod::Options => axum::routing::options(handler),
    };

    if let Some(cors_config) = cors_override {
        let sub = Router::<AppState>::new()
            .route(path, method_router)
            .layer(build_cors_layer(cors_config));
        app.merge(sub)
    } else {
        app.route(path, method_router)
    }
}

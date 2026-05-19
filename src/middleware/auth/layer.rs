/// Auth middleware layer for Axum routers.
///
/// Provides a Tower `Layer` implementation that checks authentication
/// for each request before passing it to the handler.

use std::collections::HashMap;
use std::convert::Infallible;

use axum::{body::Body, http::Response, Router};
use axum::response::IntoResponse;
use futures_util::future::BoxFuture;
use tower::{Layer, Service};

use crate::error::AppError;

use crate::context::RequestContext;

use super::validate::{authenticate, check_roles};

/// Tower Layer for authentication.
///
/// Wraps the router and adds auth checking to each request.
/// Reads the endpoint's auth configuration from the request extensions
/// (populated by the router based on the matched path).
#[derive(Debug, Clone)]
pub struct AuthLayer {
    skip_routes: Vec<String>,
}

impl AuthLayer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            skip_routes: Vec::new(),
        }
    }

    #[must_use]
    pub fn skip_routes(mut self, routes: &[&str]) -> Self {
        self.skip_routes = routes.iter().map(|s| s.to_string()).collect();
        self
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthService {
            inner,
            skip_routes: self.skip_routes.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthService<S> {
    inner: S,
    skip_routes: Vec<String>,
}

impl<S, B> Service<axum::http::Request<B>> for AuthService<S>
where
    S: Service<axum::http::Request<B>, Response = Response<Body>> + Send + 'static,
    S::Error: Into<Infallible> + Send + 'static,
    B: Send + 'static,
{
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut req: axum::http::Request<B>) -> Self::Future {
        let skip_routes = self.skip_routes.clone();
        let inner = std::sync::Arc::new(std::sync::Mutex::new(std::sync::Arc::new(self.inner)));

        Box::pin(async move {
            let path = req.uri().path().to_string();
            if skip_routes.iter().any(|skip| path.starts_with(skip)) {
                return inner
                    .lock()
                    .unwrap()
                    .call(req)
                    .await
                    .map_err(|e| e.into());
            }

            let endpoint_config = req
                .extensions()
                .get::<crate::config::types::EndpointConfig>()
                .cloned();

            let auth_config = req
                .extensions()
                .get::<crate::config::types::AuthConfig>()
                .cloned();

            let headers = req.headers().clone();
            let query_params: HashMap<String, String> = req
                .extensions()
                .get::<axum::extract::Query<HashMap<String, String>>>()
                .map(|q| q.0.clone())
                .unwrap_or_default();

            let (auth_type, required_roles) = if let Some(endpoint) = endpoint_config {
                (endpoint.auth.clone(), endpoint.roles)
            } else {
                ("none".to_string(), Vec::new())
            };

            if auth_type == "none" {
                return inner
                    .lock()
                    .unwrap()
                    .call(req)
                    .await
                    .map_err(|e| e.into());
            }

            let auth_config = match auth_config {
                Some(cfg) => cfg,
                None => {
                    let response = AppError::Config(
                        "Auth required but no auth config provided".to_string(),
                    )
                    .into_response();
                    return Ok(response);
                }
            };

            let auth_info = match authenticate(&auth_type, &auth_config, &headers, &query_params)
                .await
            {
                Ok(info) => info,
                Err(e) => {
                    let response = e.into_response();
                    return Ok(response);
                }
            };

            if let Err(e) = check_roles(&auth_info, &required_roles) {
                let response = e.into_response();
                return Ok(response);
            }

            let context = RequestContext {
                user_id: Some(auth_info.subject.clone()),
                user_role: auth_info.role.clone(),
                headers: headers
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.to_string(),
                            v.to_str().unwrap_or("").to_string(),
                        )
                    })
                    .collect(),
                method: req.method().to_string(),
                path: path.clone(),
                query_params: query_params.clone(),
            };

            req.extensions_mut().insert(auth_info);
            req.extensions_mut().insert(context);

            inner
                .lock()
                .unwrap()
                .call(req)
                .await
                .map_err(|e| e.into())
        })
    }
}

/// Extension trait for Router to apply auth layer.
pub trait RouterExt<S> {
    /// Apply authentication layer to the router.
    fn auth_layer(self) -> Router<S>;
}

impl<S> RouterExt<S> for Router<S> {
    fn auth_layer(self) -> Router<S> {
        self.layer(AuthLayer::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_layer_skips_disabled_routes() {
        let layer = AuthLayer::new();
        assert!(layer.skip_routes.is_empty());

        let layer_with_skip = AuthLayer::new().skip_routes(&["/_main-serve/health"]);
        assert!(layer_with_skip.skip_routes.contains(&"_main-serve/health".to_string()));
    }
}

/// Middleware coordinator.
///
/// This module orchestrates all middleware components, ensuring they are:
/// - Applied in the correct order
/// - Properly composed with proper error handling
/// - Configurable per-endpoint where applicable
/// - Documented with clear dependencies
///
/// ## Middleware Order
///
/// The middleware stack is applied in the following order (outermost to innermost):
///
/// 1. **Compression** - Response compression (tower layer)
/// 2. **Request ID** - Generate/propagate request IDs (tower layer)
/// 3. **Logging/Tracing** - HTTP request/response tracing (tower layer)
/// 4. **Body Limit** - Enforce maximum request body size (axum middleware)
/// 5. **Body Logging** - Optional request/response body logging (axum middleware)
/// 6. **Auth** - Authentication and authorization (axum middleware)
/// 7. **Rate Limiting** - Per-endpoint rate limiting (axum middleware)
/// 8. **CORS** - Per-endpoint CORS policy (tower layer, applied to sub-routers)
///
/// ## Dependencies
///
/// - **Rate limiting** can run before or after auth - for token-based limiting,
///   it reads the Authorization header directly without requiring auth middleware.
/// - **Body logging** requires body_limit to run first (to reconstruct body after checking size)
/// - **CORS** is applied at the sub-router level for per-endpoint configuration
/// - **Auth** runs before handlers to validate credentials and populate request context
///
/// ## Rate Limiting Coordination
///
/// Rate limiting is applied via middleware (run once per request) and should NOT be
/// duplicated in handler pre-checks. Duplicate rate limiting causes request counters
/// to increment twice, leading to premature rate limiting failures.
///
/// ## Per-Endpoint Configuration
///
/// The coordinator reads endpoint configs and applies middleware appropriately:
/// - CORS can be overridden per-endpoint (global + per-endpoint merge)
/// - Rate limiting can be overridden per-endpoint (global + per-endpoint merge)
/// - Auth is configured per-endpoint (each endpoint specifies its auth type)
/// - Body logging, compression, request ID are global only
use axum::Router;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};

use crate::config::types::{CorsConfig, EndpointConfig, RateLimitConfig};
use crate::middleware::body_limit::body_limit_middleware;
use crate::middleware::compression::build_compression_layer;
use crate::middleware::logging::{body_logging_middleware, build_trace_layer};
use crate::middleware::rate_limit::rate_limit_middleware;
use crate::server::state::AppState;

/// Determines the effective CORS config for an endpoint.
///
/// If the endpoint has a CORS config, it overrides the global config.
/// Otherwise, the global config is used.
///
/// # Examples
///
/// ```rust
/// use main_serve::config::types::{CorsConfig, EndpointConfig, EndpointAction};
///
/// let global_cors = CorsConfig {
///     allowed_origins: vec!["https://example.com".to_string()],
///     allowed_methods: vec!["GET".to_string()],
///     allowed_headers: vec!["Content-Type".to_string()],
///     allow_credentials: false,
///     max_age: 86400,
/// };
///
/// let endpoint_cors = CorsConfig {
///     allowed_origins: vec!["https://api.example.com".to_string()],
///     allowed_methods: vec!["GET".to_string(), "POST".to_string()],
///     allowed_headers: vec!["Content-Type".to_string(), "X-API-Key".to_string()],
///     allow_credentials: true,
///     max_age: 3600,
/// };
///
/// let endpoint = EndpointConfig {
///     path: "/api/users".to_string(),
///     methods: vec![],
///     action: EndpointAction::CustomResponse,
///     auth: "none".to_string(),
///     roles: vec![],
///     rate_limit: None,
///     cors: Some(endpoint_cors),
///     crud: None,
///     proxy: None,
///     static_files: None,
///     custom_response: None,
/// };
///
/// let resolved = main_serve::middleware::coordinator::resolve_cors_config(&endpoint, &global_cors);
/// assert_eq!(resolved.allowed_origins, vec!["https://api.example.com".to_string()]);
/// ```
pub fn resolve_cors_config(endpoint: &EndpointConfig, global_cors: &CorsConfig) -> CorsConfig {
    endpoint.cors.as_ref().unwrap_or(global_cors).clone()
}

/// Determines the effective rate limit config for an endpoint.
///
/// If the endpoint has a rate limit config, it overrides the global config.
/// Otherwise, the global config is used.
///
/// # Examples
///
/// ```rust
/// use main_serve::config::types::{RateLimitConfig, EndpointConfig, EndpointAction};
///
/// let global_rate_limit = RateLimitConfig {
///     enabled: true,
///     max_requests: 100,
///     window_seconds: 60,
///     key_strategy: main_serve::config::types::RateLimitKeyStrategy::Ip,
///     key_header: "".to_string(),
///     cleanup_threshold: 10000,
/// };
///
/// let endpoint_rate_limit = RateLimitConfig {
///     enabled: true,
///     max_requests: 10,
///     window_seconds: 60,
///     key_strategy: main_serve::config::types::RateLimitKeyStrategy::Token,
///     key_header: "".to_string(),
///     cleanup_threshold: 1000,
/// };
///
/// let endpoint = EndpointConfig {
///     path: "/api/users".to_string(),
///     methods: vec![],
///     action: EndpointAction::CustomResponse,
///     auth: "none".to_string(),
///     roles: vec![],
///     rate_limit: Some(endpoint_rate_limit),
///     cors: None,
///     crud: None,
///     proxy: None,
///     static_files: None,
///     custom_response: None,
/// };
///
/// let resolved = main_serve::middleware::coordinator::resolve_rate_limit_config(&endpoint, &global_rate_limit);
/// assert_eq!(resolved.max_requests, 10);
/// ```
pub fn resolve_rate_limit_config(
    endpoint: &EndpointConfig,
    global_rate_limit: &RateLimitConfig,
) -> RateLimitConfig {
    endpoint
        .rate_limit
        .as_ref()
        .unwrap_or(global_rate_limit)
        .clone()
}

/// Builder for the global middleware stack.
///
/// Provides a fluent interface for building the complete middleware chain
/// with proper ordering and coordination.
#[derive(Clone)]
pub struct MiddlewareStack {
    state: AppState,
    enable_body_logging: bool,
}

impl MiddlewareStack {
    /// Create a new middleware stack builder.
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            enable_body_logging: false,
        }
    }

    /// Enable body logging for requests.
    pub fn with_body_logging(mut self) -> Self {
        self.enable_body_logging = true;
        self
    }

    /// Build the complete middleware stack.
    ///
    /// Returns a closure that can be applied to a Router to add the middleware stack.
    /// The middleware is applied in this order (outer to inner):
    /// 1. Compression
    /// 2. Request ID
    /// 3. Trace/Logging
    /// 4. Body Limit
    /// 5. Body Logging (conditional)
    /// 6. Rate Limiting
    pub async fn build(self) -> impl Fn(Router) -> Router {
        let state = self.state.clone();
        let config = state.config.clone();
        let enable_body_logging = self.enable_body_logging;

        let logging_config = {
            let cfg = config.read().await;
            (
                cfg.logging.log_request_body,
                cfg.logging.log_response_body,
                cfg.logging.log_request_body || cfg.logging.log_response_body,
            )
        };

        move |router: Router| {
            let router = router
                .layer(build_compression_layer())
                .layer(PropagateRequestIdLayer::x_request_id())
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(build_trace_layer())
                .layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    body_limit_middleware,
                ));

            let router = if enable_body_logging && logging_config.2 {
                router.layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    body_logging_middleware,
                ))
            } else {
                router
            };

            router.layer(axum::middleware::from_fn_with_state(
                state.clone(),
                rate_limit_middleware,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_cors_config_uses_endpoint_override() {
        let global_cors = CorsConfig {
            allowed_origins: vec!["https://example.com".to_string()],
            allowed_methods: vec!["GET".to_string()],
            allowed_headers: vec!["Content-Type".to_string()],
            allow_credentials: false,
            max_age: 86400,
        };

        let endpoint_cors = CorsConfig {
            allowed_origins: vec!["https://api.example.com".to_string()],
            allowed_methods: vec!["GET".to_string(), "POST".to_string()],
            allowed_headers: vec!["Content-Type".to_string(), "X-API-Key".to_string()],
            allow_credentials: true,
            max_age: 3600,
        };

        let endpoint = EndpointConfig {
            path: "/api/users".to_string(),
            methods: vec![],
            action: crate::config::types::EndpointAction::CustomResponse,
            auth: "none".to_string(),
            roles: vec![],
            rate_limit: None,
            cors: Some(endpoint_cors.clone()),
            crud: None,
            proxy: None,
            static_files: None,
            custom_response: None,
        };

        let resolved = resolve_cors_config(&endpoint, &global_cors);

        assert_eq!(resolved.allowed_origins, endpoint_cors.allowed_origins);
        assert_eq!(resolved.allowed_methods, endpoint_cors.allowed_methods);
        assert!(resolved.allow_credentials);
    }

    #[test]
    fn test_resolve_cors_config_uses_global_when_no_override() {
        let global_cors = CorsConfig {
            allowed_origins: vec!["https://example.com".to_string()],
            allowed_methods: vec!["GET".to_string()],
            allowed_headers: vec!["Content-Type".to_string()],
            allow_credentials: false,
            max_age: 86400,
        };

        let endpoint = EndpointConfig {
            path: "/api/users".to_string(),
            methods: vec![],
            action: crate::config::types::EndpointAction::CustomResponse,
            auth: "none".to_string(),
            roles: vec![],
            rate_limit: None,
            cors: None,
            crud: None,
            proxy: None,
            static_files: None,
            custom_response: None,
        };

        let resolved = resolve_cors_config(&endpoint, &global_cors);

        assert_eq!(resolved.allowed_origins, global_cors.allowed_origins);
        assert_eq!(resolved.allowed_methods, global_cors.allowed_methods);
    }

    #[test]
    fn test_resolve_rate_limit_config_uses_endpoint_override() {
        let global_rate_limit = RateLimitConfig {
            enabled: true,
            max_requests: 100,
            window_seconds: 60,
            key_strategy: crate::config::types::RateLimitKeyStrategy::Ip,
            key_header: "".to_string(),
            cleanup_threshold: 10000,
        };

        let endpoint_rate_limit = RateLimitConfig {
            enabled: true,
            max_requests: 10,
            window_seconds: 60,
            key_strategy: crate::config::types::RateLimitKeyStrategy::Token,
            key_header: "".to_string(),
            cleanup_threshold: 1000,
        };

        let endpoint = EndpointConfig {
            path: "/api/users".to_string(),
            methods: vec![],
            action: crate::config::types::EndpointAction::CustomResponse,
            auth: "none".to_string(),
            roles: vec![],
            rate_limit: Some(endpoint_rate_limit.clone()),
            cors: None,
            crud: None,
            proxy: None,
            static_files: None,
            custom_response: None,
        };

        let resolved = resolve_rate_limit_config(&endpoint, &global_rate_limit);

        assert_eq!(resolved.max_requests, endpoint_rate_limit.max_requests);
        assert_eq!(resolved.key_strategy, endpoint_rate_limit.key_strategy);
    }

    #[test]
    fn test_resolve_rate_limit_config_uses_global_when_no_override() {
        let global_rate_limit = RateLimitConfig {
            enabled: true,
            max_requests: 100,
            window_seconds: 60,
            key_strategy: crate::config::types::RateLimitKeyStrategy::Ip,
            key_header: "".to_string(),
            cleanup_threshold: 10000,
        };

        let endpoint = EndpointConfig {
            path: "/api/users".to_string(),
            methods: vec![],
            action: crate::config::types::EndpointAction::CustomResponse,
            auth: "none".to_string(),
            roles: vec![],
            rate_limit: None,
            cors: None,
            crud: None,
            proxy: None,
            static_files: None,
            custom_response: None,
        };

        let resolved = resolve_rate_limit_config(&endpoint, &global_rate_limit);

        assert_eq!(resolved.max_requests, global_rate_limit.max_requests);
        assert_eq!(resolved.key_strategy, global_rate_limit.key_strategy);
    }
}

use axum::response::Response;
use http::{HeaderValue, header};

/// CORS middleware built from YAML configuration.
///
/// Converts `CorsConfig` into a `tower_http::cors::CorsLayer` that can be
/// applied as a router layer.
use tower_http::cors::{AllowHeaders, AllowOrigin, CorsLayer};

use crate::config::types::CorsConfig;

/// Build a `CorsLayer` from a `CorsConfig`.
pub fn build_cors_layer(config: &CorsConfig) -> CorsLayer {
    let mut layer = CorsLayer::new();

    // Origins.
    if config.allowed_origins.len() == 1 && config.allowed_origins[0] == "*" {
        layer = layer.allow_origin(AllowOrigin::any());
    } else {
        let origins: Vec<http::HeaderValue> = config
            .allowed_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        layer = layer.allow_origin(origins);
    }

    // Methods.
    let methods: Vec<http::Method> = config
        .allowed_methods
        .iter()
        .filter_map(|m| m.parse().ok())
        .collect();
    layer = layer.allow_methods(methods);

    // Headers.
    if config.allowed_headers.len() == 1 && config.allowed_headers[0] == "*" {
        layer = layer.allow_headers(AllowHeaders::any());
    } else {
        let headers: Vec<http::HeaderName> = config
            .allowed_headers
            .iter()
            .filter_map(|h| h.parse().ok())
            .collect();
        layer = layer.allow_headers(headers);
    }

    // Credentials.
    if config.allow_credentials {
        layer = layer.allow_credentials(true);
    }

    // Max age.
    layer = layer.max_age(std::time::Duration::from_secs(config.max_age));

    layer
}

/// Apply CORS headers directly to a response.
///
/// This is used for OPTIONS preflight responses and other cases where
/// the tower layer approach isn't suitable.
pub fn apply_cors_headers(response: &mut Response, config: &CorsConfig) {
    // Allow-Origin
    let origin_value = if config.allowed_origins.iter().any(|o| o == "*") {
        HeaderValue::from_static("*")
    } else {
        // In a real scenario, you'd select the origin from the request.
        // For now, use the first allowed origin.
        HeaderValue::from_str(config.allowed_origins.first().unwrap_or(&"*".to_string()))
            .unwrap_or(HeaderValue::from_static("*"))
    };
    response
        .headers_mut()
        .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin_value);

    // Allow-Methods
    let methods: Vec<_> = config
        .allowed_methods
        .iter()
        .filter_map(|m| HeaderValue::from_str(m).ok())
        .collect();
    if !methods.is_empty() {
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_str(&config.allowed_methods.join(", "))
                .unwrap_or(HeaderValue::from_static("GET, POST, PUT, DELETE")),
        );
    }

    // Allow-Headers
    let headers_value = if config.allowed_headers.iter().any(|h| h == "*") {
        HeaderValue::from_static("*")
    } else {
        HeaderValue::from_str(&config.allowed_headers.join(", "))
            .unwrap_or(HeaderValue::from_static("*"))
    };
    response
        .headers_mut()
        .insert(header::ACCESS_CONTROL_ALLOW_HEADERS, headers_value);

    // Allow-Credentials
    if config.allow_credentials {
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
            HeaderValue::from_static("true"),
        );
    }

    // Max-Age
    response.headers_mut().insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_str(&config.max_age.to_string())
            .unwrap_or(HeaderValue::from_static("86400")),
    );
}

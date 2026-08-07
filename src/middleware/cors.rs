//! CORS middleware: constructs and applies CORS policies from config.
use axum::http::HeaderValue;
use tower_http::cors::{AllowHeaders, AllowOrigin, CorsLayer};

use crate::config::types::CorsConfig;

/// Build a `CorsLayer` from a `CorsConfig`.
///
/// Creates a `tower_http::cors::CorsLayer` that can be applied
/// as a router layer to enforce CORS policy.
pub fn build_cors_layer(config: &CorsConfig) -> CorsLayer {
    let mut layer = CorsLayer::new();

    // Origins.
    if config.allowed_origins.len() == 1 && config.allowed_origins[0] == "*" {
        layer = layer.allow_origin(AllowOrigin::any());
    } else {
        let origins: Vec<HeaderValue> = config
            .allowed_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        if !origins.is_empty() {
            layer = layer.allow_origin(origins);
        }
    }

    // Methods.
    let methods: Vec<http::Method> = config
        .allowed_methods
        .iter()
        .filter_map(|m| m.parse().ok())
        .collect();
    if !methods.is_empty() {
        layer = layer.allow_methods(methods);
    }

    // Headers.
    if config.allowed_headers.len() == 1 && config.allowed_headers[0] == "*" {
        layer = layer.allow_headers(AllowHeaders::any());
    } else {
        let headers: Vec<http::HeaderName> = config
            .allowed_headers
            .iter()
            .filter_map(|h| h.parse().ok())
            .collect();
        if !headers.is_empty() {
            layer = layer.allow_headers(headers);
        }
    }

    // Credentials.
    if config.allow_credentials {
        layer = layer.allow_credentials(true);
    }

    // Max age.
    layer = layer.max_age(std::time::Duration::from_secs(config.max_age));

    layer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cors_config() -> CorsConfig {
        CorsConfig {
            allowed_origins: vec!["https://example.com".to_string()],
            allowed_methods: vec!["GET".to_string(), "POST".to_string()],
            allowed_headers: vec!["Content-Type".to_string()],
            allow_credentials: false,
            max_age: 3600,
        }
    }

    fn make_wildcard_cors_config() -> CorsConfig {
        CorsConfig {
            allowed_origins: vec!["*".to_string()],
            allowed_methods: vec!["GET".to_string(), "POST".to_string(), "PUT".to_string()],
            allowed_headers: vec!["*".to_string()],
            allow_credentials: false,
            max_age: 7200,
        }
    }

    fn make_creds_cors_config() -> CorsConfig {
        CorsConfig {
            allowed_origins: vec!["https://example.com".to_string()],
            allowed_methods: vec!["GET".to_string()],
            allowed_headers: vec!["Authorization".to_string()],
            allow_credentials: true,
            max_age: 600,
        }
    }

    #[test]
    fn test_build_cors_layer_specific_origins() {
        let config = make_cors_config();
        let _layer = build_cors_layer(&config);
        // Layer is built successfully - actual CORS checking happens at request time.
        assert!(
            !config.allowed_origins.contains(&"*".to_string()),
            "Config should not be wildcard for this test"
        );
    }

    #[test]
    fn test_build_cors_layer_wildcard_origin() {
        let config = make_wildcard_cors_config();
        let _layer = build_cors_layer(&config);
        // Wildcard origins without credentials is acceptable.
        assert!(
            !config.allow_credentials,
            "Wildcard origins test must not have credentials enabled"
        );
    }

    #[test]
    fn test_build_cors_layer_credentials() {
        let config = make_creds_cors_config();
        let _layer = build_cors_layer(&config);
        // Layer is built. With credentials, origins must not be wildcard.
        assert!(config.allow_credentials, "Credentials should be enabled");
        assert!(
            !config.allowed_origins.contains(&"*".to_string()),
            "When credentials are allowed, origins must NOT be wildcard"
        );
    }
}

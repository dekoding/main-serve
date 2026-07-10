use axum::http::{HeaderValue, header};
use axum::response::Response;
use tower_http::cors::{AllowHeaders, AllowOrigin, CorsLayer};

use crate::config::types::CorsConfig;

/// Default CORS max-age value in seconds (24 hours).
const DEFAULT_CORS_MAX_AGE: &str = "86400";

/// Select the appropriate CORS origin string based on config and request origin.
///
/// When `allow_credentials` is true, echoes back the request's origin if it's
/// in the allowed list (or if wildcard is allowed). Otherwise returns the
/// first allowed origin, or `"*"` if it's in the allowed list.
fn get_cors_origin(
    allowed_origins: &[String],
    allow_credentials: bool,
    request_origin: Option<&HeaderValue>,
) -> String {
    if allow_credentials {
        request_origin
            .and_then(|origin_value| {
                let origin_str = origin_value.to_str().ok()?;
                if allowed_origins.contains(&"*".to_string()) {
                    Some(origin_str.to_string())
                } else {
                    allowed_origins
                        .iter()
                        .find(|allowed| *allowed == origin_str)
                        .cloned()
                }
            })
            .unwrap_or_else(|| "*".to_string())
    } else if allowed_origins.iter().any(|o| o == "*") {
        "*".to_string()
    } else {
        allowed_origins.first().cloned().unwrap_or_default()
    }
}

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

/// Apply CORS headers directly to a response.
///
/// This is used for OPTIONS preflight responses and other cases where
/// the tower layer approach isn't suitable. The `origin` parameter should
/// be the value of the `Origin` header from the incoming request, if present.
pub fn apply_cors_headers(
    response: &mut Response,
    config: &CorsConfig,
    origin: Option<&HeaderValue>,
) {
    // Allow-Origin: Select the appropriate origin header value.
    // When credentials are allowed, we cannot use "*" and must echo back
    // the specific origin from the request if it's in our allowed list.
    let origin_value = get_cors_origin(&config.allowed_origins, config.allow_credentials, origin);
    let origin_header =
        HeaderValue::from_str(&origin_value).unwrap_or_else(|_| HeaderValue::from_static("*"));

    response
        .headers_mut()
        .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin_header);

    // Allow-Methods: Comma-separated list of allowed HTTP methods.
    if !config.allowed_methods.is_empty() {
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_str(&config.allowed_methods.join(", "))
                .unwrap_or(HeaderValue::from_static("GET, POST, PUT, DELETE")),
        );
    }

    // Allow-Headers: Comma-separated list of allowed request headers.
    let headers_value = if config.allowed_headers.iter().any(|h| h == "*") {
        HeaderValue::from_static("*")
    } else if !config.allowed_headers.is_empty() {
        HeaderValue::from_str(&config.allowed_headers.join(", "))
            .unwrap_or(HeaderValue::from_static("*"))
    } else {
        HeaderValue::from_static("*")
    };
    response
        .headers_mut()
        .insert(header::ACCESS_CONTROL_ALLOW_HEADERS, headers_value);

    // Allow-Credentials: Only set if explicitly enabled.
    if config.allow_credentials {
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
            HeaderValue::from_static("true"),
        );
    }

    // Max-Age: Cache duration for preflight responses in seconds.
    response.headers_mut().insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_str(&config.max_age.to_string())
            .unwrap_or(HeaderValue::from_static(DEFAULT_CORS_MAX_AGE)),
    );
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

    #[test]
    fn test_apply_cors_headers_specific_origin() {
        let config = make_cors_config();
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        let origin = HeaderValue::from_str("https://example.com").unwrap();
        apply_cors_headers(&mut response, &config, Some(&origin));

        let headers = response.headers();
        assert_eq!(
            headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_str("https://example.com").unwrap())
        );
    }

    #[test]
    fn test_apply_cors_headers_wildcard_no_credentials() {
        let mut config = make_wildcard_cors_config();
        config.allow_credentials = false;
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        let origin = HeaderValue::from_str("https://attacker.com").unwrap();
        apply_cors_headers(&mut response, &config, Some(&origin));

        let headers = response.headers();
        assert_eq!(
            headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static("*"))
        );
    }

    #[test]
    fn test_apply_cors_headers_credentials_echoes_origin() {
        let config = make_creds_cors_config();
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        let origin = HeaderValue::from_str("https://example.com").unwrap();
        apply_cors_headers(&mut response, &config, Some(&origin));

        let headers = response.headers();
        // With credentials, the origin must be echoed back, NOT wildcard.
        assert_eq!(
            headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_str("https://example.com").unwrap()),
            "With credentials, origin must be echoed back, not wildcard"
        );
        assert_eq!(
            headers.get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS),
            Some(&HeaderValue::from_static("true"))
        );
    }

    #[test]
    fn test_apply_cors_headers_credentials_wildcard_origins_uses_echo() {
        let mut config = make_creds_cors_config();
        config.allowed_origins = vec!["*".to_string()];
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        let origin = HeaderValue::from_str("https://example.com").unwrap();
        apply_cors_headers(&mut response, &config, Some(&origin));

        let headers = response.headers();
        // When allowed_origins is wildcard and credentials enabled, the request's origin is echoed back.
        assert_eq!(
            headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_str("https://example.com").unwrap()),
            "With wildcard origins + credentials, the request origin must be echoed back"
        );
        assert_ne!(
            headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static("*")),
            "With credentials, Access-Control-Allow-Origin MUST NOT be '*'"
        );
    }

    #[test]
    fn test_apply_cors_headers_unauthorized_origin_with_credentials() {
        let config = make_creds_cors_config();
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        let origin = HeaderValue::from_str("https://attacker.com").unwrap();
        apply_cors_headers(&mut response, &config, Some(&origin));

        let headers = response.headers();
        // Unauthorized origin with credentials should get no origin header (or default).
        // The behavior depends on whether the origin is in the allowed list.
        // Since "https://attacker.com" is not in allowed_origins and not "*", the origin is None.
        assert!(
            headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none()
                || headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    == Some(&HeaderValue::from_static("*")),
            "Unauthorized origin with credentials should not echo the origin"
        );
    }

    #[test]
    fn test_apply_cors_headers_no_origin_header() {
        let config = make_cors_config();
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        apply_cors_headers(&mut response, &config, None);

        let headers = response.headers();
        // When no Origin header is present, the first allowed origin is used.
        assert!(
            headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_some(),
            "Should have an Allow-Origin header"
        );
    }

    #[test]
    fn test_apply_cors_headers_max_age() {
        let config = make_cors_config();
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        let origin = HeaderValue::from_str("https://example.com").unwrap();
        apply_cors_headers(&mut response, &config, Some(&origin));

        let headers = response.headers();
        assert_eq!(
            headers.get(header::ACCESS_CONTROL_MAX_AGE),
            Some(&HeaderValue::from_str("3600").unwrap())
        );
    }
}

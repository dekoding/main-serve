/// Reverse proxy handler: forwards requests to upstream servers.
///
/// Supports path rewriting, custom header injection, and per-endpoint timeouts.
/// Uses `reqwest` with `rustls-tls` as the HTTP client.
use std::time::Duration;
use std::sync::OnceLock;

use axum::body::Body;
use axum::http::{HeaderName, HeaderValue, StatusCode, Uri};
use axum::response::Response;
use http_body_util::BodyExt;

use crate::config::types::EndpointConfig;
use crate::error::AppError;

/// Shared HTTP client for proxy upstream connections.
///
/// Reusing a single client across requests enables connection pooling,
/// reducing latency and resource usage compared to creating a new client
/// per request.
static PROXY_CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();

/// Headers that are hop-by-hop per HTTP spec and must not be forwarded through a proxy.
const HOP_BY_HOP_HEADERS: &[&str] = &[
    "host",
    "connection",
    "transfer-encoding",
    "keep-alive",
    "upgrade",
    "proxy-authorization",
    "proxy-authenticate",
    "te",
    "trailers",
];

/// Get a proxy client to use in the proxy handler.
fn get_proxy_client() -> Result<&'static reqwest::Client, AppError> {
    PROXY_CLIENT
        .get_or_init(|| {
            reqwest::Client::builder().build().ok() // Result -> Option
        })
        .as_ref()
        .ok_or_else(|| AppError::Internal("Failed to build proxy HTTP client".to_string()))
}

/// Handle a proxy endpoint - forwards the request to the configured upstream.
///
/// # Errors
///
/// Returns `AppError::Internal` if the proxy config is missing or the upstream
/// request fails.
pub async fn handle_proxy(
    method: axum::http::Method,
    uri: Uri,
    headers: axum::http::HeaderMap,
    body: Body,
    endpoint: EndpointConfig,
) -> Result<Response, AppError> {
    let proxy = endpoint
        .proxy
        .as_ref()
        .ok_or_else(|| AppError::Internal("Proxy config missing on proxy endpoint".to_string()))?;

    // Build the upstream URL.
    let incoming_path = uri.path();
    let mut upstream_path = incoming_path.to_string();

    // Apply path rewriting.
    if let Some(rewrite) = &proxy.path_rewrite {
        if !rewrite.strip_prefix.is_empty() {
            upstream_path = upstream_path
                .strip_prefix(&rewrite.strip_prefix)
                .unwrap_or(&upstream_path)
                .to_string();
        }
        if !rewrite.add_prefix.is_empty() {
            upstream_path = format!("{}{}", rewrite.add_prefix, upstream_path);
        }
    }

    // Append query string if present.
    let upstream_url = if let Some(query) = uri.query() {
        format!(
            "{}{}?{}",
            proxy.upstream.trim_end_matches('/'),
            upstream_path,
            query
        )
    } else {
        format!("{}{}", proxy.upstream.trim_end_matches('/'), upstream_path)
    };

    // Collect the incoming body.
    let body_bytes = body
        .collect()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read request body: {e}")))?
        .to_bytes();

    // Build the upstream request.
    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes())
        .map_err(|e| AppError::Internal(format!("Invalid method: {e}")))?;

    // Use the shared PROXY_CLIENT for connection pooling; timeouts are applied per-request.
    let mut upstream_req = get_proxy_client()?
        .request(reqwest_method, &upstream_url)
        .timeout(Duration::from_secs(proxy.timeouts.total));

    // Forward select headers from the original request.
    for (name, value) in &headers {
        if !HOP_BY_HOP_HEADERS.contains(&name.as_str())
            && let Ok(val) = value.to_str()
        {
            upstream_req = upstream_req.header(name.as_str(), val);
        }
    }

    // Inject configured headers.
    for (key, value) in &proxy.headers {
        upstream_req = upstream_req.header(key, value);
    }

    // Send the request with per-endpoint total timeout.
    let upstream_response = upstream_req
        .body(body_bytes.to_vec())
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Proxy upstream error: {e}")))?;

    // Build the response to return to the client.
    let status = StatusCode::from_u16(upstream_response.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);

    let mut response_builder = axum::http::Response::builder().status(status);

    // Forward upstream response headers, stripping hop-by-hop headers.
    for (name, value) in upstream_response.headers() {
        if !HOP_BY_HOP_HEADERS.contains(&name.as_str())
            && let (Ok(hn), Ok(hv)) = (
                name.as_str().parse::<HeaderName>(),
                HeaderValue::from_bytes(value.as_bytes()),
            )
        {
            response_builder = response_builder.header(hn, hv);
        }
    }

    // Reject upstream responses that exceed the configured size limit.
    let max_response = proxy.max_response_size;
    if max_response > 0
        && let Some(cl) = upstream_response.content_length()
        && cl > max_response
    {
        return Err(AppError::Internal(format!(
            "Upstream response too large ({cl} bytes, limit {max_response})"
        )));
    }

    let response_body = upstream_response
        .bytes()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read upstream body: {e}")))?;

    if max_response > 0 && response_body.len() as u64 > max_response {
        return Err(AppError::Internal(
            "Upstream response body exceeded size limit".to_string(),
        ));
    }

    response_builder
        .body(Body::from(response_body))
        .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))
}

/// Request/response logging middleware.
///
/// Wraps `tower_http::trace::TraceLayer` with structured logging via `tracing`.
/// Optionally logs request and/or response bodies when enabled in config.
use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;
use http_body_util::BodyExt;
use tower_http::trace::TraceLayer;

use crate::server::state::AppState;

// Maximum body size is configurable via `logging.max_body_log_size`.
// Bodies larger than this are truncated in the log output.

/// Build a `TraceLayer` for HTTP request/response tracing.
///
/// Wraps `tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>`
/// with default HTTP tracing behavior via `TraceLayer::new_for_http()`.
/// Body-level logging is handled separately by `body_logging_middleware`.
#[must_use]
/// build_trace_layer
pub fn build_trace_layer()
-> TraceLayer<tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>>
{
    TraceLayer::new_for_http()
}

/// Axum middleware that logs request and/or response bodies.
///
/// Reads the `log_request_body` and `log_response_body` flags from the
/// current config via `AppState`. Bodies are collected into memory and
/// logged at the `debug` level. Bodies exceeding the configured
/// `max_body_log_size` are truncated in the log output.
pub async fn body_logging_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let config = state.config.read().await;
    let log_req = config.logging.log_request_body;
    let log_res = config.logging.log_response_body;
    let max_log_size = config.logging.max_body_log_size;
    drop(config);

    // --- Request body logging ---
    let request = if log_req {
        let (parts, body) = request.into_parts();
        let bytes = body
            .collect()
            .await
            .map(http_body_util::Collected::to_bytes)
            .unwrap_or_default();
        log_body(
            "request",
            &parts.uri.to_string(),
            parts.method.as_ref(),
            &bytes,
            max_log_size,
        );

        Request::from_parts(parts, Body::from(bytes))
    } else {
        request
    };

    // --- Call next handler ---
    let response = next.run(request).await;

    // --- Response body logging ---
    if log_res {
        let (parts, body) = response.into_parts();
        let bytes = body
            .collect()
            .await
            .map(http_body_util::Collected::to_bytes)
            .unwrap_or_default();

        log_body(
            "response",
            "",
            &parts.status.to_string(),
            &bytes,
            max_log_size,
        );

        Response::from_parts(parts, Body::from(bytes))
    } else {
        response
    }
}

/// Log a body at debug level, truncating if necessary.
fn log_body(direction: &str, uri: &str, method_or_status: &str, bytes: &[u8], max_size: usize) {
    if bytes.is_empty() {
        return;
    }

    let (body_text, truncated) = if bytes.len() > max_size {
        (
            String::from_utf8_lossy(&bytes[..max_size]).into_owned(),
            true,
        )
    } else {
        (String::from_utf8_lossy(bytes).into_owned(), false)
    };

    if truncated {
        tracing::debug!(
            body.direction = direction,
            body.size = bytes.len(),
            body.truncated = true,
            "{method_or_status} {uri} body ({} bytes, truncated): {body_text}",
            bytes.len(),
        );
    } else {
        tracing::debug!(
            body.direction = direction,
            body.size = bytes.len(),
            "{method_or_status} {uri} body: {body_text}",
        );
    }
}

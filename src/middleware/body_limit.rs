use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use crate::error::AppError;
use crate::server::state::AppState;

/// Middleware that enforces a maximum request body size.
///
/// Checks the Content-Length header and rejects with 413 if exceeded.
/// If the header is not present, the body is read to check size, then
/// reconstructed so downstream handlers can consume it.
pub async fn body_limit_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    // Get the maximum body size from config
    let config = state.config.read().await;
    let max_size = config.server.max_body_size;
    drop(config);

    // First check: see if Content-Length header is present and exceeds limit.
    // If it's within limit, pass the request through without consuming the body.
    // This allows downstream handlers/middleware to consume the body normally.
    if let Some(content_length) = request
        .headers()
        .get(http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
    {
        if content_length > max_size {
            return Err(AppError::Body(format!(
                "Request body size {} exceeds maximum allowed size {}",
                content_length, max_size
            )));
        }
        // Content-Length is within limit, proceed to handler.
        // Body will be consumed by downstream middleware/handlers.
        return Ok(next.run(request).await);
    }

    // Content-Length not present (e.g., chunked encoding, tests, or axum test client).
    // Must read the body to check size, then reconstruct it.
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|e| AppError::Body(e.to_string()))?;

    if bytes.len() > max_size {
        return Err(AppError::Body(format!(
            "Request body size {} exceeds maximum allowed size {}",
            bytes.len(),
            max_size
        )));
    }

    // Reconstruct the request with the body from the bytes so downstream
    // handlers can consume it.
    let request = http::Request::from_parts(parts, Body::from(bytes));

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;
    use axum::http::HeaderValue;
    use axum::{Router, routing::get};
    use tower::ServiceExt;

    fn make_state(max_body_size: usize) -> AppState {
        let config = crate::config::types::AppConfig {
            server: crate::config::types::ServerConfig {
                max_body_size,
                ..Default::default()
            },
            ..Default::default()
        };
        AppState::new(
            config,
            std::path::PathBuf::from("/dev/null"),
            "admin-token".to_string(),
        )
    }

    async fn test_middleware(
        max_body_size: usize,
        body: Bytes,
        content_length: Option<u64>,
    ) -> Result<axum::http::Response<axum::body::Body>, AppError> {
        let state = make_state(max_body_size);
        let router = Router::new().route("/test", get(|| async { "ok" })).layer(
            axum::middleware::from_fn_with_state(state.clone(), body_limit_middleware),
        );

        let mut req = axum::http::Request::get("/test")
            .body(axum::body::Body::from(body.clone()))
            .unwrap();

        if let Some(cl) = content_length {
            req.headers_mut()
                .insert(http::header::CONTENT_LENGTH, HeaderValue::from(cl));
        }

        let resp = router.oneshot(req).await.unwrap();

        let status = resp.status();
        let (parts, body) = resp.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        let resp = axum::http::Response::from_parts(parts, Body::from(bytes));

        if status.is_success() {
            Ok(resp)
        } else {
            Err(AppError::Body(format!("Unexpected status: {}", status)))
        }
    }

    async fn test_middleware_error(
        max_body_size: usize,
        body: Bytes,
        content_length: Option<u64>,
    ) -> Result<axum::http::Response<axum::body::Body>, AppError> {
        let state = make_state(max_body_size);
        let router = Router::new().route("/test", get(|| async { "ok" })).layer(
            axum::middleware::from_fn_with_state(state.clone(), body_limit_middleware),
        );

        let mut req = axum::http::Request::get("/test")
            .body(axum::body::Body::from(body.clone()))
            .unwrap();

        if let Some(cl) = content_length {
            req.headers_mut()
                .insert(http::header::CONTENT_LENGTH, HeaderValue::from(cl));
        }

        let resp = router.oneshot(req).await.unwrap();

        let status = resp.status();
        let (parts, body) = resp.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        let resp = axum::http::Response::from_parts(parts, Body::from(bytes));

        if status.is_success() {
            Ok(resp)
        } else {
            Err(AppError::Body(format!("Unexpected status: {}", status)))
        }
    }

    #[tokio::test]
    async fn test_body_limit_within_content_length() {
        let result = test_middleware(1024, Bytes::from(vec![0u8; 100]), Some(100)).await;
        assert!(result.is_ok(), "Request within limit should be accepted");
    }

    #[tokio::test]
    async fn test_body_limit_exceeds_content_length() {
        let result = test_middleware_error(100, Bytes::from(vec![0u8; 200]), Some(200)).await;
        assert!(
            result.is_err(),
            "Request exceeding limit should be rejected"
        );
    }

    #[tokio::test]
    async fn test_body_limit_no_content_length_within() {
        let result = test_middleware(1024, Bytes::from(vec![0u8; 50]), None).await;
        assert!(
            result.is_ok(),
            "Request without Content-Length within limit should be accepted"
        );
    }

    #[tokio::test]
    async fn test_body_limit_no_content_length_exceeds() {
        let result = test_middleware_error(50, Bytes::from(vec![0u8; 200]), None).await;
        assert!(
            result.is_err(),
            "Request without Content-Length exceeding limit should be rejected"
        );
    }

    #[tokio::test]
    async fn test_body_limit_exact_boundary() {
        let result = test_middleware(100, Bytes::from(vec![0u8; 100]), Some(100)).await;
        assert!(
            result.is_ok(),
            "Request at exact boundary should be accepted"
        );
    }
}

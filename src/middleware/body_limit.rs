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

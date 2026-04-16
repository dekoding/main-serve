/// Unified error types for the application.
///
/// All HTTP error responses follow the format:
/// ```json
/// { "error": { "code": "...", "message": "..." } }
/// ```
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Application-wide error type.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Authentication error: {0}")]
    AuthChallenge(String, String),

    #[error("Authorization error: {0}")]
    Forbidden(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Rate limited")]
    RateLimited,

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: String,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            AppError::Config(_) => (StatusCode::INTERNAL_SERVER_ERROR, "config_error"),
            AppError::Validation(_) => (StatusCode::BAD_REQUEST, "validation_error"),
            AppError::Database(e) => {
                tracing::error!("Database error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "database_error")
            }
            AppError::Auth(_) => (StatusCode::UNAUTHORIZED, "auth_error"),
            AppError::AuthChallenge(_, _) => (StatusCode::UNAUTHORIZED, "auth_error"),
            AppError::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            AppError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
            AppError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, "io_error"),
        };

        let body = ErrorBody {
            error: ErrorDetail {
                code: code.to_string(),
                message: match &self {
                    // Sanitize database errors to avoid leaking connection strings or SQL.
                    AppError::Database(_) => "A database error occurred".to_string(),
                    other => other.to_string(),
                },
            },
        };

        let mut response = (status, axum::Json(body)).into_response();

        // If this is a basic auth challenge, include the WWW-Authenticate header.
        if let AppError::AuthChallenge(_, ref challenge) = self
            && let Ok(val) = http::HeaderValue::from_str(challenge)
        {
            response
                .headers_mut()
                .insert(http::header::WWW_AUTHENTICATE, val);
        }

        response
    }
}

/// Unified error types for the application.
///
/// All HTTP error responses follow the format:
/// ```json
/// { "error": { "code": "...", "message": "..." } }
/// ```
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::config::types::HttpMethod;

/// Application-wide error type.
///
/// Each variant documents whether it represents a user-facing error (typically 4xx)
/// or a system/internal error (typically 5xx). This distinction determines the HTTP
/// status code and whether the error message should be returned to the client.
#[derive(Debug, thiserror::Error)]
/// `AppError`
pub enum AppError {
    /// Configuration error (internal / 500).
    ///
    /// The configuration YAML is syntactically valid and well-formed, but contains
    /// values that are semantically incorrect or reference resources that do not exist.
    /// These are system-level problems with the server's configuration.
    ///
    /// # Examples
    /// - A path in the config points to a non-existent directory: `root: "/nonexistent/path"`
    /// - A database URL references a host that is unreachable
    /// - A TLS certificate file path is specified but the file does not exist
    /// - A storage backend configuration references a store name that is not defined
    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    /// Validation error (internal / 500).
    ///
    /// The configuration YAML contains syntactic or structural mistakes that prevent
    /// it from being parsed or understood. These are system-level problems with the
    /// server's configuration.
    ///
    /// # Examples
    /// - A required field is missing from the YAML
    /// - A value has the wrong type (e.g., a string where an integer is expected)
    /// - An unknown field is present in the YAML
    /// - A value is outside its allowed range or enum
    #[error("Validation error: {0}")]
    Validation(String),

    /// Database error (internal / 500).
    ///
    /// An error originating from the database layer, such as a query failure,
    /// connection loss, or constraint violation at runtime. The database error
    /// message is sanitized before being returned to the client to avoid leaking
    /// connection strings, table names, or SQL internals.
    ///
    /// # Examples
    /// - A SQL query fails due to a runtime constraint
    /// - The database connection is lost mid-request
    /// - A deadlock occurs during a transaction
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Authentication error (client / 401).
    ///
    /// The request failed to prove the caller's identity. This covers missing,
    /// malformed, expired, or invalid credentials. The client can typically
    /// recover by providing valid authentication.
    ///
    /// # Examples
    /// - A JWT token is expired or has an invalid signature
    /// - An API key is not in the allowed list
    /// - Basic auth credentials are incorrect
    #[error("Authentication error: {0}")]
    Auth(String),

    /// Authentication challenge (client / 401).
    ///
    /// A variant of `Auth` used when the server needs to include a
    /// `WWW-Authenticate` header in the response (e.g., for Basic auth).
    /// Contains both the error message and the challenge string.
    ///
    /// # Examples
    /// - Returning a Basic realm challenge because no Authorization header was sent
    /// - Returning a Basic realm challenge because the credentials were rejected
    #[error("Authentication error: {0}")]
    AuthChallenge(String, String),

    /// Authorization error (client / 403).
    ///
    /// The caller is authenticated but does not have permission to perform
    /// the requested action. The client cannot recover without a role change
    /// or resource reassignment.
    ///
    /// # Examples
    /// - A user with only the role "reader" tries to write to an endpoint that requires the "editor" role
    /// - A user tries to access a resource owned by another user
    #[error("Authorization error: {0}")]
    Forbidden(String),

    /// Not found (client / 404).
    ///
    /// The requested resource does not exist at the specified path or identifier.
    ///
    /// # Examples
    /// - A GET request for a user ID that does not exist
    /// - A file path that does not exist in the storage backend
    #[error("Not found: {0}")]
    NotFound(String),

    /// Method not allowed (client / 405).
    ///
    /// The HTTP method is not permitted for the requested resource.
    ///
    /// # Examples
    /// - Sending a DELETE request to an endpoint that only supports GET and POST
    #[error("Method not allowed: {message}")]
    MethodNotAllowed {
        /// Human-readable error message.
        message: String,
        /// Allowed HTTP methods for this endpoint, used to populate the `Allow` response header.
        allowed: Vec<HttpMethod>,
    },

    /// Rate limited (client / 429).
    ///
    /// The client has exceeded the allowed number of requests within the
    /// configured time window. The response should include a `Retry-After`
    /// header indicating when the client can retry.
    #[error("Rate limited")]
    RateLimited(Option<u64>),

    /// Bad request (client / 400).
    ///
    /// The request body or parameters are malformed or missing required fields.
    /// The client can typically recover by correcting the request.
    ///
    /// # Examples
    /// - A JSON body fails to parse
    /// - A required form field is missing
    /// - A query parameter has an invalid format
    #[error("Bad request: {0}")]
    BadRequest(String),

    /// JSONB column validation failure (client / 400).
    ///
    /// One or more JSONB column values in the request body failed JSON Schema
    /// validation. The error includes a summary message and a structured list
    /// of per-column violation details so clients can highlight specific fields.
    ///
    /// # Examples
    /// - A JSONB `metadata` column is missing a required `title` property
    /// - A JSONB `tags` array contains non-string elements
    #[error("JSONB validation failed: {message}")]
    JsonValidationError {
        /// Summary message describing the first or most important validation failure.
        message: String,
        /// Per-column violation messages. Empty when there is only one error.
        details: Vec<String>,
    },

    /// Payload too large (client / 413).
    ///
    /// The request body or file upload exceeds the configured size limit.
    ///
    /// # Examples
    /// - A file upload exceeds the endpoint's max upload size
    /// - A request body exceeds the server's `max_body_size`
    #[error("Payload too large: {0}")]
    PayloadTooLarge(String),

    /// Service unavailable (internal / 503).
    ///
    /// The server is temporarily unable to handle the request, typically due
    /// to an upstream dependency being down or a resource being exhausted.
    ///
    /// # Examples
    /// - An S3 backend is unreachable
    /// - All database connections are in use
    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    /// File operation error (internal / 500).
    ///
    /// An error during a file or storage operation, such as a read, write,
    /// delete, or rename failing at the storage backend level.
    ///
    /// # Examples
    /// - A file cannot be written due to permissions
    /// - A directory cannot be created because the parent does not exist
    /// - A file rename fails because the source does not exist
    #[error("File operation error: {0}")]
    FileOperation(String),

    /// Conflict (client / 409).
    ///
    /// The request conflicts with the current state of the server. The client
    /// can typically recover by modifying the request (e.g., deleting attached
    /// entities first, or waiting for a resource to become available).
    ///
    /// # Examples
    /// - Deleting media that is attached to content entities when `on_delete: error`
    /// - Creating a resource that already exists
    #[error("Conflict: {message}")]
    Conflict {
        /// Human-readable error message.
        message: String,
        /// Details about the conflict (e.g., list of attached entity references).
        details: Vec<String>,
    },

    /// Internal error (internal / 500).
    ///
    /// An unexpected error that should not occur in normal operation. This is
    /// a catch-all for programmer errors, logic bugs, or unexpected states.
    /// The error message is returned to the client as-is, so it should not
    /// contain sensitive information.
    ///
    /// # Examples
    /// - A regex match fails unexpectedly (should never happen with a static pattern)
    /// - A required configuration field is missing at runtime (programming bug)
    /// - An invariant is violated in the query builder
    #[error("Internal error: {0}")]
    Internal(String),

    /// IO error (internal / 500).
    ///
    /// A low-level operating system IO error, such as a file descriptor failure
    /// or a network socket error.
    ///
    /// # Examples
    /// - A file cannot be opened due to OS-level restrictions
    /// - A TCP socket fails to write data
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Body extraction error (client / 413).
    ///
    /// An error extracting the request body, typically because the body is
    /// too large or cannot be read. This maps to 413 to indicate that the
    /// payload itself is the problem.
    ///
    /// # Examples
    /// - The request body exceeds the configured limit during extraction
    /// - The body cannot be read due to a transport error
    #[error("Body extraction error: {0}")]
    Body(String),

    /// Parse error (client / 400).
    ///
    /// A request body or parameter failed to parse into the expected format.
    /// The client can typically recover by providing correctly formatted input.
    ///
    /// # Examples
    /// - A JSON body is not valid JSON
    /// - A query parameter cannot be parsed as an integer
    #[error("Parse error: {0}")]
    ParseError(String),

    /// Requested range not satisfiable (client / 416).
    ///
    /// The client requested a byte range that exceeds the available data.
    /// This is returned when serving partial content via range requests.
    ///
    /// # Examples
    /// - A GET with `Range: bytes=500-600` on a 100-byte file
    #[error("Requested range not satisfiable: {message}")]
    RequestedRangeNotSatisfiable {
        /// Human-readable error message.
        message: String,
        /// Optional `Content-Range: bytes */{total}` header value to include in the response.
        content_range: Option<String>,
    },
}

#[derive(Serialize)]
/// JSON response body containing the error code and message.
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
/// Error detail with a machine-readable code and human-readable message.
struct ErrorDetail {
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Vec<String>>,
}

#[derive(Serialize)]
/// JSON response body for JSONB validation errors with structured per-column details.
struct JsonValidationErrorBody {
    error: JsonValidationErrorDetail,
}

#[derive(Serialize)]
/// Structured JSONB validation error with a summary and per-column details.
struct JsonValidationErrorDetail {
    code: String,
    message: String,
    details: Vec<String>,
}

impl IntoResponse for AppError {
    /// Converts the error into an HTTP response with appropriate status code and JSON body.
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            Self::ConfigurationError(e) => {
                tracing::error!("Configuration error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "config_error")
            }
            Self::Validation(e) => {
                tracing::error!("Validation error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "validation_error")
            }
            Self::Database(e) => {
                tracing::error!("Database error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "database_error")
            }
            Self::Auth(_) | Self::AuthChallenge(_, _) => (StatusCode::UNAUTHORIZED, "auth_error"),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            Self::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            Self::RateLimited(_) => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            Self::BadRequest(_) | Self::JsonValidationError { .. } => {
                (StatusCode::BAD_REQUEST, "bad_request")
            }
            Self::Internal(e) => {
                tracing::error!("Internal error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
            Self::Io(e) => {
                tracing::error!("IO error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "io_error")
            }
            Self::Conflict { .. } => (StatusCode::CONFLICT, "conflict"),
            Self::MethodNotAllowed { .. } => (StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed"),
            Self::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large"),
            Self::ServiceUnavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "service_unavailable"),
            Self::FileOperation(e) => {
                tracing::error!("File operation error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "file_operation")
            }
            Self::Body(_) => (StatusCode::PAYLOAD_TOO_LARGE, "request_body_error"),
            Self::ParseError(_) => (StatusCode::BAD_REQUEST, "json_parse_error"),
            Self::RequestedRangeNotSatisfiable { .. } => {
                (StatusCode::RANGE_NOT_SATISFIABLE, "range_not_satisfiable")
            }
        };

        // JSONB validation errors use a dedicated response body with structured details.
        if let Self::JsonValidationError { message, details } = self {
            let body = JsonValidationErrorBody {
                error: JsonValidationErrorDetail {
                    code: "bad_request".to_string(),
                    message,
                    details,
                },
            };
            return (status, axum::Json(body)).into_response();
        }

        let (details, message) = match &self {
            // Sanitize database errors to avoid leaking connection strings or SQL.
            Self::Database(_) => (None, "A database error occurred".to_string()),
            // Sanitize other internal errors to avoid leaking paths, connection strings, or internals.
            Self::ConfigurationError(_)
            | Self::Validation(_)
            | Self::FileOperation(_)
            | Self::Internal(_)
            | Self::Io(_) => (None, "An internal error occurred".to_string()),
            Self::Conflict {
                details, message, ..
            } => (Some(details.clone()), message.clone()),
            other => (None, other.to_string()),
        };

        let body = ErrorBody {
            error: ErrorDetail {
                code: code.to_string(),
                message,
                details,
            },
        };

        let mut response = (status, axum::Json(body)).into_response();
        add_error_headers(&self, &mut response);
        response
    }
}

/// Add protocol-specific headers to an error response (WWW-Authenticate, Retry-After, Allow, Content-Range).
fn add_error_headers(error: &AppError, response: &mut Response) {
    if let AppError::AuthChallenge(_, challenge) = error
        && let Ok(val) = http::HeaderValue::from_str(challenge)
    {
        response
            .headers_mut()
            .insert(http::header::WWW_AUTHENTICATE, val);
    }

    if let AppError::RateLimited(retry_secs) = error
        && let Some(secs) = retry_secs
        && let Ok(val) = http::HeaderValue::from_str(&secs.to_string())
    {
        response
            .headers_mut()
            .insert(http::header::RETRY_AFTER, val);
    }

    if let AppError::MethodNotAllowed { allowed, .. } = error
        && !allowed.is_empty()
    {
        let allow_value = allowed
            .iter()
            .map(HttpMethod::as_str)
            .collect::<Vec<&str>>()
            .join(", ");
        if let Ok(val) = http::HeaderValue::from_str(&allow_value) {
            response.headers_mut().insert(http::header::ALLOW, val);
        }
    }

    if let AppError::RequestedRangeNotSatisfiable { content_range, .. } = error
        && let Some(cr) = content_range
        && let Ok(val) = http::HeaderValue::from_str(cr)
    {
        response
            .headers_mut()
            .insert(http::header::CONTENT_RANGE, val);
    }
}

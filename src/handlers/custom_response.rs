/// Custom/static response handler for fixed JSON, HTML, or template responses.
///
/// Returns a pre-configured response body, status code, content type, and headers.
/// For 3xx redirect responses, returns a response with the correct status code and Location header.
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::config::types::EndpointConfig;
use crate::error::AppError;
use crate::handlers::common::utils::apply_content_type;

/// Handle a `custom_response` endpoint - returns the fixed response defined in config.
///
/// # Errors
///
/// Returns `AppError::Internal` if the custom_response config is missing.
pub async fn handle_custom_response(endpoint: EndpointConfig) -> Result<Response, AppError> {
    let Some(cr) = &endpoint.custom_response else {
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            "custom_response config missing",
        )
            .into_response());
    };

    let status = StatusCode::from_u16(cr.status).unwrap_or(StatusCode::OK);

    let mut response = (status, cr.body.clone()).into_response();

    // For 3xx redirects, set the Location header.
    if status.is_redirection()
        && let Some(location) = cr
            .headers
            .get("Location")
            .or_else(|| cr.headers.get("location"))
        && let Ok(val) = HeaderValue::from_str(location)
    {
        response
            .headers_mut()
            .insert(HeaderName::from_static("location"), val);
    }

    // Set content-type header.
    apply_content_type(&mut response, &cr.content_type);

    // Set additional custom headers.
    for (key, value) in &cr.headers {
        if let (Ok(name), Ok(val)) = (key.parse::<HeaderName>(), HeaderValue::from_str(value)) {
            response.headers_mut().insert(name, val);
        }
    }

    Ok(response)
}

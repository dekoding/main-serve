/// Custom/static response handler for fixed JSON, HTML, or template responses.
///
/// Returns a pre-configured response body, status code, content type, and headers.
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::config::types::EndpointConfig;
use crate::error::AppError;

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

    // Set content-type header.
    if let Ok(val) = HeaderValue::from_str(&cr.content_type) {
        response
            .headers_mut()
            .insert(http::header::CONTENT_TYPE, val);
    }

    // Set additional custom headers.
    for (key, value) in &cr.headers {
        if let (Ok(name), Ok(val)) = (key.parse::<HeaderName>(), HeaderValue::from_str(value)) {
            response.headers_mut().insert(name, val);
        }
    }

    Ok(response)
}

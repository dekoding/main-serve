//! CORS configuration validation.

use crate::config::types::CorsConfig;

/// Validate a single `CorsConfig`, appending errors to `errors`.
pub fn validate_cors(cors: &CorsConfig, label: &str, errors: &mut Vec<String>) {
    if cors.allowed_origins.iter().any(|o| o == "*") && cors.allow_credentials {
        errors.push(format!(
            "{label}: allowed_origins contains \"*\" but allow_credentials is true \
             — these are mutually exclusive (RFC 6454)"
        ));
    }
}

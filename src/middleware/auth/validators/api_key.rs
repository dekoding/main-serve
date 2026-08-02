//! API key credential validation.
//!
//! Looks up the API key from the configured location (header name or query
//! parameter) and validates it against the list of known keys. Returns the
//! associated role if the key is found.
use axum::http::HeaderMap;
use std::collections::HashMap;
use subtle::ConstantTimeEq;

use crate::config::types::{ApiKeyConfig, ApiKeyLocation};
use crate::error::AppError;

/// Validate an API key from the request and return the associated role.
///
/// # Errors
///
/// Returns `AppError::Auth` if the key is missing or invalid.
///
/// The `implicit_hasher` allow is needed because the function receives a
/// `&HashMap<String, String>` parameter. While this function itself only
/// iterates over the map (no `.get()` calls), the public parameter type
/// triggers the lint. Using `HashMap<String, String, RandomState>` explicitly
/// in the signature would be verbose without adding safety.
#[allow(clippy::implicit_hasher)] // public param is &HashMap<String, String> triggers lint
pub(crate) fn validate_api_key(
    headers: &HeaderMap,
    query_params: &HashMap<String, String>,
    config: &ApiKeyConfig,
) -> Result<Option<String>, AppError> {
    let key = match config.location {
        ApiKeyLocation::Header => headers
            .get(&config.name)
            .and_then(|v| v.to_str().ok())
            .map(ToString::to_string),
        ApiKeyLocation::Query => query_params.get(&config.name).cloned(),
    };

    let key = key.ok_or_else(|| AppError::Auth("API key missing".to_string()))?;

    // Find the matching key entry using constant-time comparison to prevent timing attacks.
    let entry = config
        .keys
        .iter()
        .find(|e| e.key.as_bytes().ct_eq(key.as_bytes()).into())
        .ok_or_else(|| AppError::Auth("Invalid API key".to_string()))?;

    Ok(entry.role.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::ApiKeyEntry;

    fn test_config() -> ApiKeyConfig {
        ApiKeyConfig {
            location: ApiKeyLocation::Header,
            name: "X-API-Key".to_string(),
            keys: vec![
                ApiKeyEntry {
                    key: "valid-key-1".to_string(),
                    role: Some("admin".to_string()),
                },
                ApiKeyEntry {
                    key: "valid-key-2".to_string(),
                    role: None,
                },
            ],
        }
    }

    #[test]
    fn test_valid_header_key_with_role() {
        let config = test_config();
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", "valid-key-1".parse().unwrap());
        let result = validate_api_key(&headers, &HashMap::new(), &config).unwrap();
        assert_eq!(result, Some("admin".to_string()));
    }

    #[test]
    fn test_valid_header_key_without_role() {
        let config = test_config();
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", "valid-key-2".parse().unwrap());
        let result = validate_api_key(&headers, &HashMap::new(), &config).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_invalid_key() {
        let config = test_config();
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", "wrong-key".parse().unwrap());
        let err = validate_api_key(&headers, &HashMap::new(), &config).unwrap_err();
        assert!(
            err.to_string().contains("Invalid API key"),
            "Expected 'Invalid API key' error, got: {err}"
        );
    }

    #[test]
    fn test_missing_key() {
        let config = test_config();
        let err = validate_api_key(&HeaderMap::new(), &HashMap::new(), &config).unwrap_err();
        assert!(
            err.to_string().contains("API key missing"),
            "Expected 'API key missing' error, got: {err}"
        );
    }

    #[test]
    fn test_query_param_key() {
        let mut config = test_config();
        config.location = ApiKeyLocation::Query;
        config.name = "api_key".to_string();

        let mut query = HashMap::new();
        query.insert("api_key".to_string(), "valid-key-1".to_string());
        let result = validate_api_key(&HeaderMap::new(), &query, &config).unwrap();
        assert_eq!(result, Some("admin".to_string()));
    }
}

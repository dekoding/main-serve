/// HTTP Basic authentication.
///
/// Decodes the Base64-encoded `Authorization: Basic <credentials>` header,
/// looks up the username, and verifies the password against stored Argon2 hashes.
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use base64::{Engine, engine::general_purpose::STANDARD};

use crate::config::types::BasicAuthConfig;
use crate::error::AppError;

/// Validate HTTP Basic credentials and return the associated role.
///
/// # Errors
///
/// Returns `AppError::Auth` if the header is malformed, the user is not found,
/// or the password does not match.
pub fn validate_basic_auth(
    auth_header: &str,
    config: &BasicAuthConfig,
) -> Result<Option<String>, AppError> {
    let encoded = auth_header
        .strip_prefix("Basic ")
        .ok_or_else(|| AppError::Auth("Invalid Basic auth header".to_string()))?;

    let decoded = String::from_utf8(
        STANDARD
            .decode(encoded)
            .map_err(|_| AppError::Auth("Invalid Base64 in Basic auth".to_string()))?,
    )
    .map_err(|_| AppError::Auth("Invalid UTF-8 in Basic auth credentials".to_string()))?;

    let (username, password) = decoded
        .split_once(':')
        .ok_or_else(|| AppError::Auth("Malformed Basic auth credentials".to_string()))?;

    // Find the user. We iterate all entries with constant-time username comparison
    // to avoid leaking which usernames are valid via timing.
    let user = config
        .users
        .iter()
        .find(|u| {
            use subtle::ConstantTimeEq;
            u.username.as_bytes().ct_eq(username.as_bytes()).into()
        })
        .ok_or_else(|| AppError::Auth("Invalid credentials".to_string()))?;

    // Verify password against Argon2 hash.
    let hash = PasswordHash::new(&user.password_hash)
        .map_err(|_| AppError::Internal("Invalid password hash in config".to_string()))?;

    Argon2::default()
        .verify_password(password.as_bytes(), &hash)
        .map_err(|_| AppError::Auth("Invalid credentials".to_string()))?;

    Ok(user.role.clone())
}

/// Extract the Basic auth value from an Authorization header.
#[must_use]
pub fn extract_basic_auth(auth_header: &str) -> Option<&str> {
    if auth_header.starts_with("Basic ") {
        Some(auth_header)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{BasicAuthConfig, BasicAuthUser};
    use argon2::{Argon2, PasswordHasher, password_hash::SaltString};

    fn hash_password(password: &str) -> String {
        let salt = SaltString::from_b64("dGVzdHNhbHR2YWx1ZQ").unwrap();
        let argon2 = Argon2::default();
        argon2
            .hash_password(password.as_bytes(), &salt)
            .unwrap()
            .to_string()
    }

    fn test_config() -> BasicAuthConfig {
        BasicAuthConfig {
            realm: "test".to_string(),
            users: vec![
                BasicAuthUser {
                    username: "admin".to_string(),
                    password_hash: hash_password("secret"),
                    role: Some("admin".to_string()),
                },
                BasicAuthUser {
                    username: "user".to_string(),
                    password_hash: hash_password("password"),
                    role: None,
                },
            ],
        }
    }

    #[test]
    fn test_valid_basic_auth() {
        let config = test_config();
        // "admin:secret" in Base64 = "YWRtaW46c2VjcmV0"
        let result = validate_basic_auth("Basic YWRtaW46c2VjcmV0", &config).unwrap();
        assert_eq!(result, Some("admin".to_string()));
    }

    #[test]
    fn test_valid_basic_auth_no_role() {
        let config = test_config();
        // "user:password" in Base64 = "dXNlcjpwYXNzd29yZA=="
        let result = validate_basic_auth("Basic dXNlcjpwYXNzd29yZA==", &config).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_wrong_password() {
        let config = test_config();
        // "admin:wrong" in Base64 = "YWRtaW46d3Jvbmc="
        let result = validate_basic_auth("Basic YWRtaW46d3Jvbmc=", &config);
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_user() {
        let config = test_config();
        // "nobody:pass" in Base64 = "bm9ib2R5OnBhc3M="
        let result = validate_basic_auth("Basic bm9ib2R5OnBhc3M=", &config);
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_basic_prefix() {
        let config = test_config();
        let result = validate_basic_auth("Bearer token123", &config);
        assert!(result.is_err());
    }
}

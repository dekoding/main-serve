/// JWT token creation and validation.
///
/// Validates JWT Bearer tokens from the `Authorization` header against the
/// configured secret, algorithm, issuer, and audience. Extracts the user's
/// role from a configurable claim.
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::config::types::{JwtAlgorithm, JwtConfig};
use crate::error::AppError;

/// Claims embedded in a JWT token.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (user identifier).
    pub sub: String,
    /// Expiration time (UTC epoch seconds).
    pub exp: usize,
    /// Issued at (UTC epoch seconds).
    pub iat: usize,
    /// Issuer.
    #[serde(default)]
    pub iss: String,
    /// Audience.
    #[serde(default)]
    pub aud: String,
    /// Role (dynamic claim name resolved by config).
    #[serde(default)]
    pub role: Option<String>,
}

/// Validate a JWT token string and extract claims.
///
/// # Errors
///
/// Returns `AppError::Auth` if the token is invalid, expired, or fails
/// issuer/audience validation.
pub fn validate_token(token: &str, config: &JwtConfig) -> Result<Claims, AppError> {
    let algorithm = map_algorithm(config.algorithm);
    let key = DecodingKey::from_secret(config.secret.as_bytes());

    let mut validation = Validation::new(algorithm);

    if config.issuer.is_empty() {
        validation.set_issuer::<String>(&[]);
    } else {
        validation.set_issuer(&[&config.issuer]);
    }

    if config.audience.is_empty() {
        validation.set_audience::<String>(&[]);
    } else {
        validation.set_audience(&[&config.audience]);
    }

    validation.validate_exp = true;

    let token_data = decode::<serde_json::Value>(token, &key, &validation)
        .map_err(|e| AppError::Auth(format!("Invalid JWT: {e}")))?;

    let claims_value = token_data.claims;

    // Extract role from the configured claim name.
    let role = claims_value
        .get(&config.role_claim)
        .and_then(|v| v.as_str())
        .map(ToString::to_string);

    let sub = claims_value
        .get("sub")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Auth("JWT 'sub' claim is missing".to_string()))
        .and_then(|s| {
            if s.trim().is_empty() {
                Err(AppError::Auth("JWT 'sub' claim is empty".to_string()))
            } else {
                Ok(s.to_string())
            }
        })?;

    let exp = claims_value
        .get("exp")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(0);

    let iat = claims_value
        .get("iat")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(0);

    let iss = claims_value
        .get("iss")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let aud = claims_value
        .get("aud")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(Claims {
        sub,
        exp,
        iat,
        iss,
        aud,
        role,
    })
}

/// Create a signed JWT token from claims.
///
/// # Errors
///
/// Returns `AppError::Internal` if the system clock is before the epoch,
/// the expiry is too large, or JWT encoding fails.
pub fn create_token(sub: &str, role: Option<&str>, config: &JwtConfig) -> Result<String, AppError> {
    let now = usize::try_from(chrono::Utc::now().timestamp())
        .map_err(|_| AppError::Internal("System clock before epoch".to_string()))?;
    let expiry = usize::try_from(config.expiry)
        .map_err(|_| AppError::Internal("Token expiry too large for this platform".to_string()))?;
    let exp = now.saturating_add(expiry);

    let mut claims_map = serde_json::json!({
        "sub": sub,
        "iat": now,
        "exp": exp,
    });

    if !config.issuer.is_empty() {
        claims_map["iss"] = serde_json::json!(config.issuer);
    }
    if !config.audience.is_empty() {
        claims_map["aud"] = serde_json::json!(config.audience);
    }
    if let Some(r) = role {
        claims_map[&config.role_claim] = serde_json::json!(r);
    }

    let algorithm = map_algorithm(config.algorithm);
    let header = Header::new(algorithm);
    let key = EncodingKey::from_secret(config.secret.as_bytes());

    encode(&header, &claims_map, &key)
        .map_err(|e| AppError::Internal(format!("JWT encode error: {e}")))
}

/// Extract the Bearer token from an Authorization header value.
#[must_use]
pub fn extract_bearer_token(auth_header: &str) -> Option<&str> {
    auth_header.strip_prefix("Bearer ")
}

fn map_algorithm(alg: JwtAlgorithm) -> Algorithm {
    match alg {
        JwtAlgorithm::HS256 => Algorithm::HS256,
        JwtAlgorithm::HS384 => Algorithm::HS384,
        JwtAlgorithm::HS512 => Algorithm::HS512,
        JwtAlgorithm::RS256 => Algorithm::RS256,
        JwtAlgorithm::RS384 => Algorithm::RS384,
        JwtAlgorithm::RS512 => Algorithm::RS512,
        JwtAlgorithm::ES256 => Algorithm::ES256,
        JwtAlgorithm::ES384 => Algorithm::ES384,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::JwtConfig;

    fn test_config() -> JwtConfig {
        JwtConfig {
            secret: "test-secret-key-that-is-long-enough".to_string(),
            algorithm: JwtAlgorithm::HS256,
            issuer: "test-issuer".to_string(),
            audience: "test-audience".to_string(),
            expiry: 3600,
            role_claim: "role".to_string(),
        }
    }

    #[test]
    fn test_create_and_validate_token() {
        let config = test_config();
        let token = create_token("user1", Some("admin"), &config).unwrap();
        let claims = validate_token(&token, &config).unwrap();
        assert_eq!(claims.sub, "user1");
        assert_eq!(claims.role, Some("admin".to_string()));
    }

    #[test]
    fn test_validate_invalid_token() {
        let config = test_config();
        let err = validate_token("invalid.token.here", &config).unwrap_err();
        assert!(
            err.to_string().contains("Invalid JWT"),
            "Expected 'Invalid JWT' error, got: {err}"
        );
    }

    #[test]
    fn test_validate_wrong_secret() {
        let config = test_config();
        let token = create_token("user1", None, &config).unwrap();

        let mut wrong_config = test_config();
        wrong_config.secret = "wrong-secret-key-that-is-different".to_string();
        let err = validate_token(&token, &wrong_config).unwrap_err();
        assert!(
            err.to_string().contains("Invalid JWT"),
            "Expected 'Invalid JWT' error, got: {err}"
        );
    }

    #[test]
    fn test_extract_bearer_token() {
        assert_eq!(extract_bearer_token("Bearer abc123"), Some("abc123"));
        assert_eq!(extract_bearer_token("Basic abc123"), None);
        assert_eq!(extract_bearer_token("abc123"), None);
    }

    #[test]
    fn test_token_without_role() {
        let config = test_config();
        let token = create_token("user1", None, &config).unwrap();
        let claims = validate_token(&token, &config).unwrap();
        assert_eq!(claims.sub, "user1");
        assert_eq!(claims.role, None);
    }

    #[test]
    fn test_validate_token_missing_sub() {
        let config = test_config();
        let now = usize::try_from(chrono::Utc::now().timestamp()).unwrap();
        let exp = now + 3600;
        let claims = serde_json::json!({
            "iat": now,
            "exp": exp,
            "iss": config.issuer,
            "aud": config.audience,
            "role": "user"
        });
        let header = Header::new(map_algorithm(config.algorithm));
        let key = EncodingKey::from_secret(config.secret.as_bytes());
        let token = encode(&header, &claims, &key).unwrap();

        let err = validate_token(&token, &config).unwrap_err();
        assert!(
            err.to_string().contains("sub' claim is missing"),
            "Expected 'sub' missing error, got: {err}"
        );
    }

    #[test]
    fn test_validate_token_empty_sub() {
        let config = test_config();
        let now = usize::try_from(chrono::Utc::now().timestamp()).unwrap();
        let exp = now + 3600;
        let claims = serde_json::json!({
            "sub": "",
            "iat": now,
            "exp": exp,
            "iss": config.issuer,
            "aud": config.audience,
            "role": "user"
        });
        let header = Header::new(map_algorithm(config.algorithm));
        let key = EncodingKey::from_secret(config.secret.as_bytes());
        let token = encode(&header, &claims, &key).unwrap();

        let err = validate_token(&token, &config).unwrap_err();
        assert!(
            err.to_string().contains("sub' claim is empty"),
            "Expected 'sub' empty error, got: {err}"
        );
    }
}

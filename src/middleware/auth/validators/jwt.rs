/// JWT token creation and validation.
///
/// Validates JWT Bearer tokens from the `Authorization` header against the
/// configured secret, algorithm, issuer, and audience. Extracts the user's
/// role from a configurable claim.
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::config::types::{JwtAlgorithm, JwtConfig};
use crate::error::AppError;
use crate::server::state::RevocationStoreBackend as RevocationStoreTrait;

/// Claims embedded in a JWT token.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Claims {
    /// Subject (user identifier - database ID).
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
    /// Unique token identifier. Used for token revocation.
    #[serde(default)]
    pub jti: Option<String>,
    /// Email address of the user.
    #[serde(default)]
    pub email: Option<String>,
}

/// Validate a JWT token string and extract claims.
///
/// If `revocation_store` is provided and the token has a `jti` claim, the
/// token is checked against the revocation store before being accepted.
///
/// # Errors
///
/// Returns `AppError::Auth` if the token is invalid, expired, or fails
/// issuer/audience validation. Returns `AppError::Auth` if the token
/// has been revoked.
pub(crate) async fn validate_token<S: RevocationStoreTrait>(
    token: &str,
    config: &JwtConfig,
    revocation_store: Option<&S>,
) -> Result<Claims, AppError> {
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

    let jti = claims_value
        .get("jti")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);

    let email = claims_value
        .get("email")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);

    if let Some(store) = revocation_store
        && let Some(ref jti) = jti
        && store.is_revoked(jti).await
    {
        return Err(AppError::Auth("Token has been revoked".to_string()));
    }

    Ok(Claims {
        sub,
        exp,
        iat,
        iss,
        aud,
        role,
        jti,
        email,
    })
}

/// Create a signed JWT token from claims.
///
/// # Errors
///
/// Returns `AppError::Internal` if the system clock is before the epoch,
/// the expiry is too large, or JWT encoding fails.
pub fn create_token(
    sub: &str,
    role: Option<&str>,
    config: &JwtConfig,
    jti: Option<&str>,
    email: Option<&str>,
) -> Result<String, AppError> {
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
    if let Some(j) = jti {
        claims_map["jti"] = serde_json::json!(j);
    }
    if let Some(e) = email {
        claims_map["email"] = serde_json::json!(e);
    }

    let algorithm = map_algorithm(config.algorithm);
    let header = Header::new(algorithm);
    let key = EncodingKey::from_secret(config.secret.as_bytes());

    encode(&header, &claims_map, &key)
        .map_err(|e| AppError::Internal(format!("JWT encode error: {e}")))
}

/// Extract the Bearer token from an Authorization header value.
#[must_use]
pub(crate) fn extract_bearer_token(auth_header: &str) -> Option<&str> {
    auth_header.strip_prefix("Bearer ")
}

/// Maps a `JwtAlgorithm` to its corresponding `jsonwebtoken::Algorithm` variant.
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
    use crate::server::state::InMemoryRevocationStore;

    fn test_config() -> JwtConfig {
        JwtConfig {
            secret: "test-secret-key-that-is-long-enough".to_string(),
            algorithm: JwtAlgorithm::HS256,
            issuer: "test-issuer".to_string(),
            audience: "test-audience".to_string(),
            expiry: 3600,
            role_claim: "role".to_string(),
            revocation: None,
        }
    }

    #[tokio::test]
    async fn test_create_and_validate_token() {
        let config = test_config();
        let token = create_token("user1", Some("admin"), &config, None, None).unwrap();
        let claims = validate_token::<InMemoryRevocationStore>(&token, &config, None)
            .await
            .unwrap();
        assert_eq!(claims.sub, "user1");
        assert_eq!(claims.role, Some("admin".to_string()));
    }

    #[tokio::test]
    async fn test_validate_invalid_token() {
        let config = test_config();
        let err = validate_token::<InMemoryRevocationStore>("invalid.token.here", &config, None)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("Invalid JWT"),
            "Expected 'Invalid JWT' error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_validate_wrong_secret() {
        let config = test_config();
        let token = create_token("user1", None, &config, None, None).unwrap();

        let mut wrong_config = test_config();
        wrong_config.secret = "wrong-secret-key-that-is-different".to_string();
        let err = validate_token::<InMemoryRevocationStore>(&token, &wrong_config, None)
            .await
            .unwrap_err();
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

    #[tokio::test]
    async fn test_token_without_role() {
        let config = test_config();
        let token = create_token("user1", None, &config, None, None).unwrap();
        let claims = validate_token::<InMemoryRevocationStore>(&token, &config, None)
            .await
            .unwrap();
        assert_eq!(claims.sub, "user1");
        assert_eq!(claims.role, None);
    }

    #[tokio::test]
    async fn test_validate_token_missing_sub() {
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

        let err = validate_token::<InMemoryRevocationStore>(&token, &config, None)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("sub' claim is missing"),
            "Expected 'sub' missing error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_validate_token_empty_sub() {
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

        let err = validate_token::<InMemoryRevocationStore>(&token, &config, None)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("sub' claim is empty"),
            "Expected 'sub' empty error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_validate_expired_token() {
        let config = test_config();
        let now = usize::try_from(chrono::Utc::now().timestamp()).unwrap();
        let claims = serde_json::json!({
            "sub": "user1",
            "iat": now - 7200,
            "exp": now - 3600,
            "iss": config.issuer,
            "aud": config.audience,
        });
        let header = Header::new(map_algorithm(config.algorithm));
        let key = EncodingKey::from_secret(config.secret.as_bytes());
        let token = encode(&header, &claims, &key).unwrap();

        let err = validate_token::<InMemoryRevocationStore>(&token, &config, None)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("Invalid JWT"),
            "Expired token should be rejected, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_validate_wrong_issuer() {
        let config = test_config();
        let token = create_token("user1", None, &config, None, None).unwrap();

        let mut wrong_config = test_config();
        wrong_config.issuer = "different-issuer".to_string();
        let err = validate_token::<InMemoryRevocationStore>(&token, &wrong_config, None)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("Invalid JWT"),
            "Token with wrong issuer should be rejected, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_validate_wrong_audience() {
        let config = test_config();
        let token = create_token("user1", None, &config, None, None).unwrap();

        let mut wrong_config = test_config();
        wrong_config.audience = "different-audience".to_string();
        let err = validate_token::<InMemoryRevocationStore>(&token, &wrong_config, None)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("Invalid JWT"),
            "Token with wrong audience should be rejected, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_none_algorithm_rejected() {
        use base64::Engine;

        let now = usize::try_from(chrono::Utc::now().timestamp()).unwrap();
        let exp = now + 3600;

        // Manually construct a JWT with "alg": "none" to test that it's rejected.
        let header_json = serde_json::json!({"alg": "none", "typ": "JWT"});
        let payload_json = serde_json::json!({
            "sub": "attacker",
            "iat": now,
            "exp": exp,
        });

        let header_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_string(&header_json).unwrap().as_bytes());
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_string(&payload_json).unwrap().as_bytes());
        let token = format!("{}.{}.{}", header_b64, payload_b64, "");

        let config = test_config();
        let err = validate_token::<InMemoryRevocationStore>(&token, &config, None)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("Invalid JWT"),
            "Token with 'alg: none' should be rejected, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_create_token_with_jti() {
        let config = test_config();
        let jti = "test-jti-12345";
        let token = create_token("user1", Some("admin"), &config, Some(jti), None).unwrap();
        let claims = validate_token::<InMemoryRevocationStore>(&token, &config, None)
            .await
            .unwrap();
        assert_eq!(claims.sub, "user1");
        assert_eq!(claims.jti, Some(jti.to_string()));
    }

    #[tokio::test]
    async fn test_create_token_without_jti() {
        let config = test_config();
        let token = create_token("user1", Some("admin"), &config, None, None).unwrap();
        let claims = validate_token::<InMemoryRevocationStore>(&token, &config, None)
            .await
            .unwrap();
        assert_eq!(claims.sub, "user1");
        assert_eq!(claims.jti, None);
    }

    #[tokio::test]
    async fn test_token_with_jti_accepted_when_not_revoked() {
        let config = test_config();
        let jti = "jti-to-test-not-revoked";
        let token = create_token("user1", Some("admin"), &config, Some(jti), None).unwrap();

        let store = InMemoryRevocationStore::default();

        let claims = validate_token::<InMemoryRevocationStore>(&token, &config, Some(&store))
            .await
            .unwrap();
        assert_eq!(claims.jti, Some(jti.to_string()));
    }

    #[tokio::test]
    async fn test_token_with_jti_rejected_when_revoked() {
        let config = test_config();
        let jti = "jti-to-test-revoked";
        let token = create_token("user1", Some("admin"), &config, Some(jti), None).unwrap();

        let store = InMemoryRevocationStore::default();
        let expires_at = std::time::Instant::now() + std::time::Duration::from_secs(3600);
        store.revoke(jti, expires_at).await;

        let err = validate_token::<InMemoryRevocationStore>(&token, &config, Some(&store))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("revoked"),
            "Expected revoked error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_token_without_jti_accepted_even_with_revocation_enabled() {
        let config = test_config();
        let store = InMemoryRevocationStore::default();

        // Token created without jti
        let token = create_token("user1", Some("user"), &config, None, None).unwrap();

        let claims = validate_token::<InMemoryRevocationStore>(&token, &config, Some(&store))
            .await
            .unwrap();
        assert_eq!(claims.sub, "user1");
        assert_eq!(claims.jti, None);
    }

    #[tokio::test]
    async fn test_revocation_does_not_affect_tokens_without_jti() {
        let config = test_config();
        let store = InMemoryRevocationStore::default();

        // Revoke a JTI that doesn't match any token
        let fake_jti = "nonexistent-jti";
        let expires_at = std::time::Instant::now() + std::time::Duration::from_secs(3600);
        store.revoke(fake_jti, expires_at).await;

        // Token without jti should still validate
        let token = create_token("user1", Some("user"), &config, None, None).unwrap();
        let claims = validate_token::<InMemoryRevocationStore>(&token, &config, Some(&store))
            .await
            .unwrap();
        assert_eq!(claims.sub, "user1");
    }

    #[tokio::test]
    async fn test_multiple_jtis_independent_revocation() {
        let config = test_config();
        let store = InMemoryRevocationStore::default();

        let jti1 = "jti-first";
        let jti2 = "jti-second";
        let token1 = create_token("user1", None, &config, Some(jti1), None).unwrap();
        let token2 = create_token("user2", None, &config, Some(jti2), None).unwrap();

        // Only revoke jti1
        let expires_at = std::time::Instant::now() + std::time::Duration::from_secs(3600);
        store.revoke(jti1, expires_at).await;

        // token1 should be rejected
        let err1 = validate_token::<InMemoryRevocationStore>(&token1, &config, Some(&store))
            .await
            .unwrap_err();
        assert!(err1.to_string().contains("revoked"));

        // token2 should still be accepted
        let claims2 = validate_token::<InMemoryRevocationStore>(&token2, &config, Some(&store))
            .await
            .unwrap();
        assert_eq!(claims2.jti, Some(jti2.to_string()));
    }

    #[tokio::test]
    async fn test_custom_role_claim_with_jti() {
        let mut config = test_config();
        config.role_claim = "custom_role".to_string();
        let jti = "jti-custom-role";
        let token = create_token("user1", Some("editor"), &config, Some(jti), None).unwrap();

        let store = InMemoryRevocationStore::default();
        let claims = validate_token::<InMemoryRevocationStore>(&token, &config, Some(&store))
            .await
            .unwrap();
        assert_eq!(claims.sub, "user1");
        assert_eq!(claims.role, Some("editor".to_string()));
        assert_eq!(claims.jti, Some(jti.to_string()));
    }
}

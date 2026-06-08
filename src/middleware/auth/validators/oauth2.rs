use base64::Engine;
/// OAuth2/OIDC authentication validation.
///
/// Provides validation functions for externally-obtained Bearer tokens.
/// The authorization code flow handlers are in `middleware::auth::handler`.
use sha2::Digest;

use std::collections::HashMap;
use std::time::Instant;

use crate::config::types::OAuth2Config;
use crate::error::AppError;

/// Pending `OAuth2` authorization flow (stored between authorize and callback).
#[derive(Debug)]
pub struct PendingOAuth2 {
    /// PKCE code verifier to include in the token exchange.
    pub code_verifier: String,
    /// When this pending state was created (for TTL expiration).
    pub created_at: Instant,
}

/// Validate an `OAuth2` access token by calling the userinfo endpoint.
///
/// Returns the user's subject and optional role from the userinfo response.
///
/// # Errors
///
/// Returns `AppError::Config` if the userinfo URL is not set.
/// Returns `AppError::Auth` if the userinfo request fails or the response
/// is missing the `sub` claim.
pub(crate) async fn validate_oauth2_token(
    token: &str,
    config: &OAuth2Config,
) -> Result<(String, Option<String>), AppError> {
    if config.userinfo_url.is_empty() {
        return Err(AppError::Config(
            "OAuth2 userinfo_url is not configured".to_string(),
        ));
    }

    fetch_userinfo(&config.userinfo_url, token).await
}

/// Fetch user information from the OIDC userinfo endpoint.
pub(crate) async fn fetch_userinfo(
    userinfo_url: &str,
    access_token: &str,
) -> Result<(String, Option<String>), AppError> {
    let client = reqwest::Client::new();
    let response = client
        .get(userinfo_url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| AppError::Auth(format!("OAuth2 userinfo request failed: {e}")))?;

    if !response.status().is_success() {
        return Err(AppError::Auth(format!(
            "OAuth2 userinfo returned status {}",
            response.status()
        )));
    }

    let userinfo: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Auth(format!("OAuth2 userinfo parse error: {e}")))?;

    let sub = userinfo
        .get("sub")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Auth("OAuth2 userinfo response missing 'sub' claim".to_string()))?
        .to_string();

    let role = userinfo
        .get("role")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);

    Ok((sub, role))
}

/// Generate a PKCE code verifier and its S256 code challenge.
///
/// Returns `(code_verifier, code_challenge)`.
#[must_use]
pub(crate) fn generate_pkce_pair() -> (String, String) {
    // 96-character code verifier using UUID v4 hex values (within the 43-128 range).
    let verifier = format!(
        "{}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple(),
    );

    let mut hasher = sha2::Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);

    (verifier, challenge)
}

/// Remove expired pending states from the map.
pub(crate) fn cleanup_expired(
    pending: &mut HashMap<String, PendingOAuth2>,
    ttl: std::time::Duration,
) {
    pending.retain(|_, v| v.created_at.elapsed() < ttl);
}

/// Exchange an authorization code for tokens at the `IdP`'s token endpoint.
pub(crate) async fn exchange_code(
    config: &OAuth2Config,
    code: &str,
    code_verifier: &str,
) -> Result<serde_json::Value, AppError> {
    if config.token_url.is_empty() {
        return Err(AppError::Config(
            "OAuth2 token_url is not configured".to_string(),
        ));
    }

    let client = reqwest::Client::new();
    let response = client
        .post(&config.token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", config.redirect_url.as_str()),
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .map_err(|e| AppError::Auth(format!("OAuth2 token exchange failed: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::warn!("OAuth2 token endpoint returned {status}: {body}");
        return Err(AppError::Auth("OAuth2 token exchange failed".to_string()));
    }

    response
        .json()
        .await
        .map_err(|e| AppError::Auth(format!("OAuth2 token response parse error: {e}")))
}

/// Extract a named cookie value from the request's Cookie header.
#[must_use]
pub(crate) fn extract_cookie(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers.get("cookie")?.to_str().ok()?;
    let prefix = format!("{name}=");
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix(&prefix) {
            if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
                return Some(value[1..value.len() - 1].to_string());
            }
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn test_extract_cookie_found() {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", "main_serve_token=abc123; Path=/".parse().unwrap());
        let result = extract_cookie(&headers, "main_serve_token");
        assert_eq!(result, Some("abc123".to_string()));
    }

    #[test]
    fn test_extract_cookie_quoted() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            "main_serve_token=\"quoted_value\"; Path=/".parse().unwrap(),
        );
        let result = extract_cookie(&headers, "main_serve_token");
        assert_eq!(result, Some("quoted_value".to_string()));
    }

    #[test]
    fn test_extract_cookie_not_found() {
        let headers = HeaderMap::new();
        let result = extract_cookie(&headers, "nonexistent");
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_cookie_no_cookie_header() {
        let mut headers = HeaderMap::new();
        headers.insert("other", "value".parse().unwrap());
        let result = extract_cookie(&headers, "main_serve_token");
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_cookie_invalid_utf8() {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", HeaderValue::from_bytes(b"\xff\xfe").unwrap());
        let result = extract_cookie(&headers, "main_serve_token");
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_cookie_multiple_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            "other=a; main_serve_token=mytoken; path=b".parse().unwrap(),
        );
        let result = extract_cookie(&headers, "main_serve_token");
        assert_eq!(result, Some("mytoken".to_string()));
    }

    #[test]
    fn test_extract_cookie_empty_name() {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", "=value".parse().unwrap());
        let result = extract_cookie(&headers, "");
        // With empty name, the prefix is empty string, so all cookies match.
        // First cookie with value "value" is returned.
        assert_eq!(result, Some("value".to_string()));
    }

    #[test]
    fn test_cleanup_expired_removes_old() {
        let mut pending = HashMap::new();
        let old = PendingOAuth2 {
            code_verifier: "old".to_string(),
            created_at: Instant::now()
                .checked_sub(std::time::Duration::from_secs(600))
                .unwrap(),
        };
        let recent = PendingOAuth2 {
            code_verifier: "recent".to_string(),
            created_at: Instant::now(),
        };
        pending.insert("old".to_string(), old);
        pending.insert("recent".to_string(), recent);

        cleanup_expired(&mut pending, std::time::Duration::from_secs(300));
        assert!(!pending.contains_key("old"), "Old entry should be removed");
        assert!(pending.contains_key("recent"), "Recent entry should remain");
    }

    #[test]
    fn test_cleanup_expired_empty_map() {
        let mut pending: HashMap<String, PendingOAuth2> = HashMap::new();
        cleanup_expired(&mut pending, std::time::Duration::from_secs(300));
        assert!(pending.is_empty());
    }

    #[test]
    fn test_cleanup_expired_all_recent() {
        let mut pending = HashMap::new();
        pending.insert(
            "a".to_string(),
            PendingOAuth2 {
                code_verifier: "v1".to_string(),
                created_at: Instant::now(),
            },
        );
        pending.insert(
            "b".to_string(),
            PendingOAuth2 {
                code_verifier: "v2".to_string(),
                created_at: Instant::now(),
            },
        );
        cleanup_expired(&mut pending, std::time::Duration::from_secs(300));
        assert_eq!(pending.len(), 2, "All entries should remain");
    }

    #[test]
    fn test_generate_pkce_pair_returns_values() {
        let (verifier, challenge) = generate_pkce_pair();
        assert!(!verifier.is_empty(), "Verifier should not be empty");
        assert!(!challenge.is_empty(), "Challenge should not be empty");
        // Verifier should be 96+ characters (3 UUIDs of 32 chars each = 96).
        assert!(
            verifier.len() >= 43,
            "Verifier length {} should be >= 43",
            verifier.len()
        );
        assert!(
            verifier.len() <= 128,
            "Verifier length {} should be <= 128",
            verifier.len()
        );
    }

    #[test]
    fn test_generate_pkce_pair_uniqueness() {
        let (v1, c1) = generate_pkce_pair();
        let (v2, c2) = generate_pkce_pair();
        assert_ne!(v1, v2, "Verifiers should be unique");
        assert_ne!(c1, c2, "Challenges should be unique");
    }
}

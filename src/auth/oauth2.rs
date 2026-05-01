/// OAuth2/OIDC authentication support.
///
/// Provides two modes of operation:
///
/// 1. **Token introspection**: Validates externally-obtained Bearer tokens
///    by calling the configured `userinfo_url`. Used when endpoints specify
///    `auth: "oauth2"`.
///
/// 2. **Authorization code flow (with PKCE)**: Handles the full `OAuth2`
///    redirect flow. Main Serve acts as an `OAuth2` client:
///    - `GET /_main-serve/oauth2/authorize` - redirects to the `IdP`
///    - `GET /_main-serve/oauth2/callback` - exchanges the code for tokens,
///      calls userinfo, mints a Main Serve JWT, sets a cookie, and redirects
///      to the configured `success_url`.
///
///    This bridges `OAuth2` into the existing JWT auth model: after the code
///    flow completes, all subsequent requests are authenticated via the
///    minted JWT (in the cookie or Authorization header).
use std::collections::HashMap;
use std::time::Instant;

use axum::extract::{Query, State};
use axum::response::Response;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

use crate::config::types::OAuth2Config;
use crate::error::AppError;
use crate::server::state::AppState;

/// Pending `OAuth2` authorization flow (stored between authorize and callback).
#[derive(Debug)]
pub struct PendingOAuth2 {
    /// PKCE code verifier to include in the token exchange.
    pub code_verifier: String,
    /// When this pending state was created (for TTL expiration).
    pub created_at: Instant,
}

/// Default lifetime for a pending `OAuth2` state (5 minutes).
/// Used as fallback when `OAuth2` config is unavailable in the callback path.
const DEFAULT_STATE_TTL_SECS: u64 = 300;

/// Generate a PKCE code verifier and its S256 code challenge.
///
/// Returns `(code_verifier, code_challenge)`.
fn generate_pkce_pair() -> (String, String) {
    // 96-character code verifier using UUID v4 hex values (within the 43-128 range).
    let verifier = format!(
        "{}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple(),
    );

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    let challenge = URL_SAFE_NO_PAD.encode(hash);

    (verifier, challenge)
}

/// Remove expired pending states from the map.
fn cleanup_expired(pending: &mut HashMap<String, PendingOAuth2>, ttl: std::time::Duration) {
    pending.retain(|_, v| v.created_at.elapsed() < ttl);
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
pub async fn validate_oauth2_token(
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

/// Handle `GET /_main-serve/oauth2/authorize`.
///
/// Generates a PKCE pair and state nonce, stores the pending state, and
/// redirects the user to the `IdP`'s authorization endpoint.
///
/// # Errors
///
/// Returns `AppError::Config` if `OAuth2` is not configured or the authorization
/// URL is empty.
pub async fn handle_oauth2_authorize(State(state): State<AppState>) -> Result<Response, AppError> {
    let config = state.config.read().await;
    let oauth2 = config
        .auth
        .oauth2
        .as_ref()
        .ok_or_else(|| AppError::Config("OAuth2 is not configured".to_string()))?;

    if oauth2.authorization_url.is_empty() {
        return Err(AppError::Config(
            "OAuth2 authorization_url is not configured".to_string(),
        ));
    }
    if oauth2.client_id.is_empty() {
        return Err(AppError::Config(
            "OAuth2 client_id is not configured".to_string(),
        ));
    }
    if oauth2.redirect_url.is_empty() {
        return Err(AppError::Config(
            "OAuth2 redirect_url is not configured".to_string(),
        ));
    }

    let state_ttl = std::time::Duration::from_secs(oauth2.state_ttl);
    let max_pending = oauth2.max_pending_states;

    let (code_verifier, code_challenge) = generate_pkce_pair();
    let state_value = uuid::Uuid::new_v4().to_string();

    // Build the authorization URL with query parameters.
    let mut url = url::Url::parse(&oauth2.authorization_url)
        .map_err(|e| AppError::Config(format!("Invalid authorization_url: {e}")))?;

    let scopes = oauth2.scopes.join(" ");
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &oauth2.client_id)
        .append_pair("redirect_uri", &oauth2.redirect_url)
        .append_pair("scope", &scopes)
        .append_pair("state", &state_value)
        .append_pair("code_challenge", &code_challenge)
        .append_pair("code_challenge_method", "S256");

    // Store the pending state for validation in the callback.
    let mut pending = state.oauth2_pending.lock().await;
    cleanup_expired(&mut pending, state_ttl);
    if pending.len() >= max_pending {
        return Err(AppError::Internal(
            "Too many pending OAuth2 authorization flows".to_string(),
        ));
    }
    pending.insert(
        state_value,
        PendingOAuth2 {
            code_verifier,
            created_at: Instant::now(),
        },
    );
    drop(pending);
    drop(config);

    axum::http::Response::builder()
        .status(axum::http::StatusCode::FOUND)
        .header("location", url.as_str())
        .body(axum::body::Body::empty())
        .map_err(|e| AppError::Internal(format!("Failed to build redirect response: {e}")))
}

/// Handle `GET /_main-serve/oauth2/callback`.
///
/// Validates the state, exchanges the authorization code for tokens at the
/// `IdP`'s token endpoint, retrieves user info, mints a Main Serve JWT,
/// sets it as an `HttpOnly` cookie, and redirects to `success_url`.
///
/// # Errors
///
/// Returns `AppError::Auth` if the `IdP` returned an error or the state is
/// invalid/expired. Returns `AppError::BadRequest` if required query parameters
/// are missing.
pub async fn handle_oauth2_callback(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    // Check for error response from the IdP.
    if let Some(error) = params.get("error") {
        let desc = params.get("error_description").map_or("", String::as_str);
        tracing::warn!("OAuth2 provider returned error: {error} {desc}");
        return Err(AppError::Auth("OAuth2 authorization failed".to_string()));
    }

    let code = params
        .get("code")
        .ok_or_else(|| AppError::BadRequest("Missing 'code' parameter".to_string()))?;
    let state_param = params
        .get("state")
        .ok_or_else(|| AppError::BadRequest("Missing 'state' parameter".to_string()))?;

    // Read the state TTL from config for expiry checks.
    let state_ttl = {
        let config = state.config.read().await;
        config.auth.oauth2.as_ref().map_or(
            std::time::Duration::from_secs(DEFAULT_STATE_TTL_SECS),
            |o| std::time::Duration::from_secs(o.state_ttl),
        )
    };

    // Look up and remove the pending state (one-time use).
    let pending_entry = {
        let mut pending = state.oauth2_pending.lock().await;
        cleanup_expired(&mut pending, state_ttl);
        pending.remove(state_param)
    };
    let pending = pending_entry
        .ok_or_else(|| AppError::Auth("Invalid or expired OAuth2 state".to_string()))?;

    if pending.created_at.elapsed() >= state_ttl {
        return Err(AppError::Auth("OAuth2 state has expired".to_string()));
    }

    let config = state.config.read().await;
    let oauth2 = config
        .auth
        .oauth2
        .as_ref()
        .ok_or_else(|| AppError::Config("OAuth2 is not configured".to_string()))?;
    let jwt_config = config.auth.jwt.as_ref().ok_or_else(|| {
        AppError::Config(
            "JWT config is required for OAuth2 code flow (used to mint tokens after login)"
                .to_string(),
        )
    })?;

    // Exchange the authorization code for tokens at the IdP.
    let token_response = exchange_code(oauth2, code, &pending.code_verifier).await?;

    let access_token = token_response
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Auth("Token response missing access_token".to_string()))?;

    // Fetch user info to get subject and role.
    let (sub, role) = if oauth2.userinfo_url.is_empty() {
        return Err(AppError::Config(
            "OAuth2 userinfo_url is required for the code flow".to_string(),
        ));
    } else {
        fetch_userinfo(&oauth2.userinfo_url, access_token).await?
    };

    // Mint a Main Serve JWT using the existing jwt config.
    let jwt = crate::auth::jwt::create_token(&sub, role.as_deref(), jwt_config)?;

    // Collect values from config before dropping the read lock.
    let success_url = if oauth2.success_url.is_empty() {
        "/".to_string()
    } else {
        oauth2.success_url.clone()
    };
    // Reject success_url values that could cause header injection via CRLF.
    if success_url.contains('\r') || success_url.contains('\n') {
        return Err(AppError::Config(
            "OAuth2 success_url contains invalid characters".to_string(),
        ));
    }
    let cookie_name = if oauth2.cookie_name.is_empty() {
        "main_serve_token".to_string()
    } else {
        oauth2.cookie_name.clone()
    };
    let has_tls = config.server.tls.is_some();
    let max_age = jwt_config.expiry;
    drop(config);

    let mut cookie_value =
        format!("{cookie_name}={jwt}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}");
    if has_tls {
        cookie_value.push_str("; Secure");
    }

    axum::http::Response::builder()
        .status(axum::http::StatusCode::FOUND)
        .header("location", &success_url)
        .header("set-cookie", &cookie_value)
        .body(axum::body::Body::empty())
        .map_err(|e| AppError::Internal(format!("Failed to build redirect response: {e}")))
}

/// Exchange an authorization code for tokens at the `IdP`'s token endpoint.
async fn exchange_code(
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

/// Fetch user information from the OIDC userinfo endpoint.
async fn fetch_userinfo(
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

/// Extract a named cookie value from the request's Cookie header.
#[must_use]
pub fn extract_cookie(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
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

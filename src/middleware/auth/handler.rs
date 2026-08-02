//! `OAuth2` authorization code flow handler.
//!
//! Handles the `OAuth2` authorization code flow:
//! - `GET /_main-serve/oauth2/authorize` - redirects to `IdP`
//! - `GET /_main-serve/oauth2/callback` - handles `IdP` callback
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::Response;

use crate::config::types::JwtConfig;
use crate::config::types::OAuth2Config;
use crate::error::AppError;
use crate::middleware::auth::validators::oauth2::{
    PendingOAuth2, cleanup_expired, exchange_code, fetch_userinfo, generate_pkce_pair,
};
use crate::server::state::AppState;

use crate::middleware::auth::validators::jwt::create_token;

/// Shared config reader for `OAuth2` operations.
///
/// Reads the `OAuth2` and JWT configuration from `AppState`, validating that
/// both are properly configured. Returns the config values cloned for
/// downstream use without holding the read lock.
///
/// # Errors
///
/// Returns `AppError::ConfigurationError` if `OAuth2` or JWT is not configured.
async fn get_oauth2_config(
    state: &AppState,
) -> Result<(Arc<OAuth2Config>, Arc<JwtConfig>), AppError> {
    let config = state.config.read().await;
    let oauth2_clone = config
        .auth
        .oauth2
        .as_ref()
        .ok_or_else(|| AppError::ConfigurationError("OAuth2 is not configured".to_string()))?
        .clone();
    let jwt_config_clone = config
        .auth
        .jwt
        .as_ref()
        .ok_or_else(|| {
            AppError::ConfigurationError(
                "JWT config is required for OAuth2 code flow (used to mint tokens after login)"
                    .to_string(),
            )
        })?
        .clone();
    drop(config);
    Ok((Arc::new(oauth2_clone), Arc::new(jwt_config_clone)))
}

/// Validate the `OAuth2` state parameter and pending entry.
///
/// Removes the pending state entry (one-time use) and verifies it has not
/// expired. Returns the `PendingOAuth2` entry for downstream use.
///
/// # Errors
///
/// Returns `AppError::Auth` if the state is missing, invalid, or expired.
async fn validate_state(
    state: &AppState,
    state_param: &str,
    state_ttl: Duration,
) -> Result<PendingOAuth2, AppError> {
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

    Ok(pending)
}

/// Exchange the authorization code for tokens at the `IdP`.
///
/// Returns the access token string extracted from the token response.
///
/// # Errors
///
/// Returns `AppError::Auth` if the token exchange fails or the response
/// is missing an access token.
async fn exchange_token(
    oauth2: &OAuth2Config,
    code: &str,
    code_verifier: &str,
) -> Result<String, AppError> {
    let token_response = exchange_code(oauth2, code, code_verifier).await?;

    token_response
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Auth("Token response missing access_token".to_string()))
        .map(std::string::ToString::to_string)
}

/// Fetch user info from the `IdP` using the access token.
///
/// Returns the subject and optional role from the userinfo endpoint.
///
/// # Errors
///
/// Returns `AppError::ConfigurationError` if the userinfo URL is empty, or
/// `AppError::Auth` if the userinfo request fails.
async fn get_userinfo(
    oauth2: &OAuth2Config,
    access_token: &str,
) -> Result<(String, Option<String>), AppError> {
    if oauth2.userinfo_url.is_empty() {
        return Err(AppError::ConfigurationError(
            "OAuth2 userinfo_url is required for the code flow".to_string(),
        ));
    }
    fetch_userinfo(
        &oauth2.userinfo_url,
        access_token,
        oauth2.role_mapping.as_ref(),
    )
    .await
}

/// Mint a Main Serve JWT for the authenticated user.
///
/// Generates a new JTI and creates a token using the provided JWT config.
///
/// # Errors
///
/// Returns `AppError::Internal` if token creation fails.
fn mint_jwt(
    sub: &str,
    role: Option<&str>,
    jwt_config: &JwtConfig,
) -> Result<(String, String), AppError> {
    let jti = uuid::Uuid::new_v4().to_string();
    let jwt = create_token(sub, role, jwt_config, Some(&jti), None)?;
    Ok((jti, jwt))
}

/// Build the redirect response with auth cookie.
///
/// Returns a 302 redirect to `success_url` with the JWT set as an
/// `HttpOnly` cookie.
///
/// # Errors
///
/// Returns `AppError::ConfigurationError` if `success_url` contains invalid characters
/// (CRLF), or `AppError::Internal` if the response builder fails.
fn build_redirect_response(
    success_url: &str,
    cookie_name: &str,
    jwt: &str,
    has_tls: bool,
    max_age: u64,
) -> Result<Response, AppError> {
    // Reject success_url values that could cause header injection via CRLF.
    if success_url.contains('\r') || success_url.contains('\n') {
        return Err(AppError::ConfigurationError(
            "OAuth2 success_url contains invalid characters".to_string(),
        ));
    }

    let mut cookie_value =
        format!("{cookie_name}={jwt}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}");
    if has_tls {
        cookie_value.push_str("; Secure");
    }

    axum::http::Response::builder()
        .status(axum::http::StatusCode::FOUND)
        .header("location", success_url)
        .header("set-cookie", &cookie_value)
        .body(axum::body::Body::empty())
        .map_err(|e| AppError::Internal(format!("Failed to build redirect response: {e}")))
}

/// Handle `GET /_main-serve/oauth2/authorize`.
///
/// Generates a PKCE pair and state nonce, stores the pending state, and
/// redirects the user to the `IdP`'s authorization endpoint.
///
/// # Errors
///
/// Returns `AppError::ConfigurationError` if `OAuth2` is not configured or the authorization
/// URL is empty.
pub async fn handle_oauth2_authorize(State(state): State<AppState>) -> Result<Response, AppError> {
    let config = state.config.read().await;
    let oauth2 = config
        .auth
        .oauth2
        .as_ref()
        .ok_or_else(|| AppError::ConfigurationError("OAuth2 is not configured".to_string()))?;

    if oauth2.authorization_url.is_empty() {
        return Err(AppError::ConfigurationError(
            "OAuth2 authorization_url is not configured".to_string(),
        ));
    }
    if oauth2.client_id.is_empty() {
        return Err(AppError::ConfigurationError(
            "OAuth2 client_id is not configured".to_string(),
        ));
    }
    if oauth2.redirect_url.is_empty() {
        return Err(AppError::ConfigurationError(
            "OAuth2 redirect_url is not configured".to_string(),
        ));
    }

    let state_ttl = std::time::Duration::from_secs(oauth2.state_ttl);
    let max_pending = oauth2.max_pending_states;

    let (code_verifier, code_challenge) = generate_pkce_pair();
    let state_value = uuid::Uuid::new_v4().to_string();

    // Build the authorization URL with query parameters.
    let mut url = url::Url::parse(&oauth2.authorization_url)
        .map_err(|e| AppError::ConfigurationError(format!("Invalid authorization_url: {e}")))?;

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
        crate::middleware::auth::validators::oauth2::PendingOAuth2 {
            code_verifier,
            created_at: std::time::Instant::now(),
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
pub async fn handle_oauth2_callback<S: std::hash::BuildHasher>(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String, S>>,
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
        config
            .auth
            .oauth2
            .as_ref()
            .map_or(std::time::Duration::from_mins(5), |o| {
                std::time::Duration::from_secs(o.state_ttl)
            })
    };

    // Step 1: Validate state and retrieve pending entry.
    let pending = validate_state(&state, state_param, state_ttl).await?;

    // Step 2: Read OAuth2 and JWT config.
    let (oauth2, jwt_config) = get_oauth2_config(&state).await?;

    // Step 3: Exchange authorization code for tokens.
    let access_token = exchange_token(&oauth2, code, &pending.code_verifier).await?;

    // Step 4: Fetch user info.
    let (sub, role) = get_userinfo(&oauth2, &access_token).await?;

    // Step 5: Mint JWT.
    let (_jti, jwt) = mint_jwt(&sub, role.as_deref(), &jwt_config)?;

    // Step 6: Build redirect response.
    let success_url = if oauth2.success_url.is_empty() {
        "/".to_string()
    } else {
        oauth2.success_url.clone()
    };
    let cookie_name = if oauth2.cookie_name.is_empty() {
        "main_serve_token".to_string()
    } else {
        oauth2.cookie_name.clone()
    };
    let has_tls = state.config.read().await.server.tls.is_some();
    let max_age = jwt_config.expiry;

    build_redirect_response(&success_url, &cookie_name, &jwt, has_tls, max_age)
}

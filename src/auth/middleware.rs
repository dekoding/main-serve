/// Auth middleware dispatcher: routes to the correct auth handler based on config.
///
/// This module provides the `authenticate` function that is called before each
/// endpoint handler. It reads the endpoint's `auth` field to determine which
/// auth provider to use, then validates the request and returns an `AuthInfo`
/// containing the authenticated user's identity and role.
use std::collections::HashMap;

use axum::http::HeaderMap;

use crate::config::types::AuthConfig;
use crate::error::AppError;

use super::api_key::validate_api_key;
use super::basic::{extract_basic_auth, validate_basic_auth};
use super::jwt::{extract_bearer_token, validate_token};
use super::oauth2::{extract_cookie, validate_oauth2_token};

/// Information about the authenticated user.
#[derive(Debug, Clone, Default)]
pub struct AuthInfo {
    /// The authenticated user's identifier (sub claim, username, key id, etc.).
    pub subject: String,
    /// The user's role, if any.
    pub role: Option<String>,
}

/// Authenticate a request based on the endpoint's auth type.
///
/// Returns `Ok(AuthInfo)` on success, or `Err(AppError::Auth)` / `Err(AppError::Forbidden)`.
/// If `auth_type` is `"none"`, returns an empty `AuthInfo` immediately.
///
/// # Errors
///
/// Returns `AppError::Config` if the auth type requires a config section that
/// is missing. Returns `AppError::Auth` or `AppError::AuthChallenge` if
/// credentials are missing or invalid.
pub async fn authenticate(
    auth_type: &str,
    auth_config: &AuthConfig,
    headers: &HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<AuthInfo, AppError> {
    match auth_type {
        "none" => Ok(AuthInfo::default()),

        "jwt" => {
            let jwt_config = auth_config.jwt.as_ref().ok_or_else(|| {
                AppError::Config("JWT auth configured but no jwt config provided".to_string())
            })?;

            // Try Authorization header first, then fall back to OAuth2 cookie.
            let token_from_header = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(extract_bearer_token)
                .map(ToString::to_string);

            let token = match token_from_header {
                Some(t) => t,
                None => {
                    // Check for JWT in the OAuth2 cookie.
                    let cookie_name = auth_config
                        .oauth2
                        .as_ref()
                        .map(|o| o.cookie_name.as_str())
                        .filter(|n| !n.is_empty());

                    match cookie_name {
                        Some(name) => extract_cookie(headers, name).ok_or_else(|| {
                            AppError::Auth(
                                "Missing Authorization header or auth cookie".to_string(),
                            )
                        })?,
                        None => {
                            return Err(AppError::Auth("Missing Authorization header".to_string()));
                        }
                    }
                }
            };

            let claims = validate_token(&token, jwt_config)?;

            Ok(AuthInfo {
                subject: claims.sub,
                role: claims.role,
            })
        }

        "api_key" => {
            let api_key_config = auth_config.api_key.as_ref().ok_or_else(|| {
                AppError::Config(
                    "API key auth configured but no api_key config provided".to_string(),
                )
            })?;

            let role = validate_api_key(headers, query_params, api_key_config)?;

            Ok(AuthInfo {
                subject: "api_key_user".to_string(),
                role,
            })
        }

        "basic" => {
            let basic_config = auth_config.basic.as_ref().ok_or_else(|| {
                AppError::Config("Basic auth configured but no basic config provided".to_string())
            })?;

            let auth_header = headers.get("authorization").and_then(|v| v.to_str().ok());

            let challenge = format!("Basic realm=\"{}\"", basic_config.realm);

            let auth_header = auth_header.ok_or_else(|| {
                AppError::AuthChallenge(
                    "Missing Authorization header".to_string(),
                    challenge.clone(),
                )
            })?;

            extract_basic_auth(auth_header).ok_or_else(|| {
                AppError::AuthChallenge("Expected Basic credentials".to_string(), challenge.clone())
            })?;

            let role = validate_basic_auth(auth_header, basic_config).map_err(|e| match e {
                AppError::Auth(msg) => AppError::AuthChallenge(msg, challenge.clone()),
                other => other,
            })?;

            Ok(AuthInfo {
                subject: "basic_user".to_string(),
                role,
            })
        }

        "oauth2" => {
            let oauth2_config = auth_config.oauth2.as_ref().ok_or_else(|| {
                AppError::Config("OAuth2 auth configured but no oauth2 config provided".to_string())
            })?;

            let auth_header = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| AppError::Auth("Missing Authorization header".to_string()))?;

            let token = extract_bearer_token(auth_header)
                .ok_or_else(|| AppError::Auth("Expected Bearer token".to_string()))?;

            let (sub, role) = validate_oauth2_token(token, oauth2_config).await?;

            Ok(AuthInfo { subject: sub, role })
        }

        other => Err(AppError::Config(format!("Unknown auth type: '{other}'"))),
    }
}

/// Check that the authenticated user has one of the required roles.
///
/// If `required_roles` is empty, all authenticated users are allowed.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the user's role is not in the required list.
pub fn check_roles(auth_info: &AuthInfo, required_roles: &[String]) -> Result<(), AppError> {
    if required_roles.is_empty() {
        return Ok(());
    }

    match &auth_info.role {
        Some(role) if required_roles.contains(role) => Ok(()),
        Some(role) => Err(AppError::Forbidden(format!(
            "Role '{role}' is not authorized for this endpoint"
        ))),
        None => Err(AppError::Forbidden(
            "No role assigned, but this endpoint requires one".to_string(),
        )),
    }
}

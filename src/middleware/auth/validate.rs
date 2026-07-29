/// Authentication validation functions used by the auth middleware.
///
/// These functions perform the actual credential validation and role checking
/// but do not contain middleware logic. They're designed to be called from
/// middleware layers or handlers.
use std::collections::{HashMap, HashSet};

use axum::http::HeaderMap;

use crate::config::types::AuthConfig;
use crate::error::AppError;
use crate::server::state::RevocationStoreBackend;

use crate::middleware::auth::validators::api_key::validate_api_key;
use crate::middleware::auth::validators::basic::{extract_basic_auth, validate_basic_auth};
use crate::middleware::auth::validators::jwt::{extract_bearer_token, validate_token};
use crate::middleware::auth::validators::oauth2::{extract_cookie, validate_oauth2_token};

/// Re-export `AuthInfo` for convenience
pub use crate::middleware::auth::extractor::AuthInfo;

/// Authenticate a request based on the endpoint's auth type.
///
/// Returns `Ok(AuthInfo)` on success, or `Err(AppError::Auth)` / `Err(AppError::Forbidden)`.
/// If `auth_type` is `"none"`, returns an empty `AuthInfo` immediately.
///
/// # Errors
///
/// Returns `AppError::ConfigurationError` if the auth type requires a config section that
/// is missing. Returns `AppError::Auth` or `AppError::AuthChallenge` if
/// credentials are missing or invalid.
///
/// The `implicit_hasher` allow is needed because the function passes
/// `query_params` to `validate_api_key` which calls `HashMap::get()`,
/// invoking the default `DefaultHasher`. An explicit `RandomState` type
/// parameter would be verbose without practical benefit for string keys.
#[allow(clippy::implicit_hasher)] // passes &HashMap to validate_api_key which uses .get()
pub async fn authenticate<'a, S: RevocationStoreBackend>(
    auth_type: &'a str,
    auth_config: &'a AuthConfig,
    headers: &'a HeaderMap,
    query_params: &'a HashMap<String, String>,
    revocation_store: Option<&'a S>,
) -> Result<AuthInfo, AppError> {
    match auth_type {
        "none" => Ok(AuthInfo::default()),

        "jwt" => {
            let jwt_config = auth_config.jwt.as_ref().ok_or_else(|| {
                AppError::ConfigurationError(
                    "JWT auth configured but no jwt config provided".to_string(),
                )
            })?;

            // Try Authorization header first, then fall back to OAuth2 cookie.
            let token_from_header = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(extract_bearer_token)
                .map(ToString::to_string);

            let token = if let Some(t) = token_from_header {
                t
            } else {
                // Check for JWT in the OAuth2 cookie.
                let cookie_name = auth_config
                    .oauth2
                    .as_ref()
                    .map(|o| o.cookie_name.as_str())
                    .filter(|n| !n.is_empty());

                match cookie_name {
                    Some(name) => extract_cookie(headers, name).ok_or_else(|| {
                        AppError::Auth("Missing Authorization header or auth cookie".to_string())
                    })?,
                    None => {
                        return Err(AppError::Auth("Missing Authorization header".to_string()));
                    }
                }
            };

            let claims = validate_token(&token, jwt_config, revocation_store).await?;

            Ok(AuthInfo {
                subject: claims.sub,
                email: claims.email,
                role: claims.role,
                token: Some(token),
            })
        }

        "api_key" => {
            let api_key_config = auth_config.api_key.as_ref().ok_or_else(|| {
                AppError::ConfigurationError(
                    "API key auth configured but no api_key config provided".to_string(),
                )
            })?;

            let role = validate_api_key(headers, query_params, api_key_config)?;

            Ok(AuthInfo {
                subject: "api_key_user".to_string(),
                email: None,
                role,
                token: None,
            })
        }

        "basic" => {
            let basic_config = auth_config.basic.as_ref().ok_or_else(|| {
                AppError::ConfigurationError(
                    "Basic auth configured but no basic config provided".to_string(),
                )
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
                email: None,
                role,
                token: None,
            })
        }

        "oauth2" => {
            let oauth2_config = auth_config.oauth2.as_ref().ok_or_else(|| {
                AppError::ConfigurationError(
                    "OAuth2 auth configured but no oauth2 config provided".to_string(),
                )
            })?;

            let auth_header = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| AppError::Auth("Missing Authorization header".to_string()))?;

            let token = extract_bearer_token(auth_header)
                .ok_or_else(|| AppError::Auth("Expected Bearer token".to_string()))?;

            let role_mapping = oauth2_config.role_mapping.as_ref();
            let (sub, role) = validate_oauth2_token(token, oauth2_config, role_mapping).await?;

            Ok(AuthInfo {
                subject: sub,
                email: None,
                role,
                token: None,
            })
        }

        other => Err(AppError::ConfigurationError(format!(
            "Unknown auth type: '{other}'"
        ))),
    }
}

/// Check that the authenticated user has one of the required roles.
///
/// If `required_roles` is empty, all authenticated users are allowed.
/// Otherwise, checks direct role membership and transitive inheritance
/// via the `role_inheritance` map (computed from the `role_hierarchy` config).
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the user's role is not in the required list
/// and does not inherit any of the required roles.
pub fn check_roles<S: std::hash::BuildHasher>(
    auth_info: &AuthInfo,
    required_roles: &[String],
    role_inheritance: &HashMap<String, HashSet<String, S>, S>,
) -> Result<(), AppError> {
    if required_roles.is_empty() {
        return Ok(());
    }

    let Some(user_role) = &auth_info.role else {
        return Err(AppError::Forbidden(
            "No role assigned, but this endpoint requires one".to_string(),
        ));
    };

    // Direct match.
    if required_roles.contains(user_role) {
        return Ok(());
    }

    // Check transitive inheritance: does the user's role inherit any required role?
    if let Some(inherited) = role_inheritance.get(user_role.as_str())
        && required_roles
            .iter()
            .any(|r| inherited.contains(r.as_str()))
    {
        return Ok(());
    }

    Err(AppError::Forbidden(format!(
        "Role '{user_role}' is not authorized for this endpoint"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_inheritance() -> HashMap<String, HashSet<String>> {
        HashMap::new()
    }

    #[test]
    fn test_check_roles_empty_allows_all() {
        let auth_info = AuthInfo {
            email: None,
            subject: "user1".to_string(),
            role: Some("user".to_string()),
            token: None,
        };
        let required_roles: Vec<String> = vec![];
        assert!(check_roles(&auth_info, &required_roles, &empty_inheritance()).is_ok());
    }

    #[test]
    fn test_check_roles_allows_matching_role() {
        let auth_info = AuthInfo {
            email: None,
            subject: "user1".to_string(),
            role: Some("admin".to_string()),
            token: None,
        };
        let required_roles = vec!["admin".to_string(), "user".to_string()];
        assert!(check_roles(&auth_info, &required_roles, &empty_inheritance()).is_ok());
    }

    #[test]
    fn test_check_roles_denies_non_matching_role() {
        let auth_info = AuthInfo {
            email: None,
            subject: "user1".to_string(),
            role: Some("guest".to_string()),
            token: None,
        };
        let required_roles = vec!["admin".to_string(), "user".to_string()];
        let result = check_roles(&auth_info, &required_roles, &empty_inheritance());
        assert!(result.is_err());
    }

    #[test]
    fn test_check_roles_denies_no_role() {
        let auth_info = AuthInfo {
            email: None,
            subject: "user1".to_string(),
            role: None,
            token: None,
        };
        let required_roles = vec!["admin".to_string()];
        let result = check_roles(&auth_info, &required_roles, &empty_inheritance());
        assert!(result.is_err());
    }

    #[test]
    fn test_check_roles_allows_inherited_role() {
        let mut inheritance = HashMap::new();
        inheritance.insert(
            "admin".to_string(),
            vec![
                "editor".to_string(),
                "author".to_string(),
                "user".to_string(),
            ]
            .into_iter()
            .collect(),
        );
        inheritance.insert(
            "editor".to_string(),
            vec!["author".to_string(), "user".to_string()]
                .into_iter()
                .collect(),
        );

        // Admin should be allowed where editor is required (via inheritance).
        let auth_info = AuthInfo {
            email: None,
            subject: "user1".to_string(),
            role: Some("admin".to_string()),
            token: None,
        };
        let required_roles = vec!["editor".to_string()];
        assert!(check_roles(&auth_info, &required_roles, &inheritance).is_ok());
    }

    #[test]
    fn test_check_roles_allows_transitively_inherited_role() {
        let mut inheritance = HashMap::new();
        // Transitive closure: admin inherits editor AND author
        inheritance.insert(
            "admin".to_string(),
            vec!["editor".to_string(), "author".to_string()]
                .into_iter()
                .collect(),
        );
        inheritance.insert(
            "editor".to_string(),
            vec!["author".to_string()].into_iter().collect(),
        );

        // Admin should be allowed where author is required (transitive).
        let auth_info = AuthInfo {
            email: None,
            subject: "user1".to_string(),
            role: Some("admin".to_string()),
            token: None,
        };
        let required_roles = vec!["author".to_string()];
        assert!(check_roles(&auth_info, &required_roles, &inheritance).is_ok());
    }

    #[test]
    fn test_check_roles_denies_inherited_role_not_in_required() {
        let mut inheritance = HashMap::new();
        inheritance.insert(
            "admin".to_string(),
            vec!["editor".to_string()].into_iter().collect(),
        );

        // Admin inherits editor, but endpoint requires "reader" (not inherited).
        let auth_info = AuthInfo {
            email: None,
            subject: "user1".to_string(),
            role: Some("admin".to_string()),
            token: None,
        };
        let required_roles = vec!["reader".to_string()];
        let result = check_roles(&auth_info, &required_roles, &inheritance);
        assert!(result.is_err());
    }

    #[test]
    fn test_check_roles_role_not_in_hierarchy_allowed_direct() {
        let inheritance = HashMap::new();

        // Role "editor" is not in the hierarchy at all, but is directly required.
        let auth_info = AuthInfo {
            email: None,
            subject: "user1".to_string(),
            role: Some("editor".to_string()),
            token: None,
        };
        let required_roles = vec!["editor".to_string()];
        assert!(check_roles(&auth_info, &required_roles, &inheritance).is_ok());
    }
}

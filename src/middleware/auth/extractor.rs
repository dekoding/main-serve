/// Auth middleware extractors for use in handlers.
///
/// Provides Axum extractors for `AuthInfo` and `RequestContext`.
use axum::{extract::FromRequestParts, http::StatusCode};
use http::request::Parts;
use std::convert::Infallible;

/// Information about the authenticated user.
///
/// Extracted by the auth middleware from request credentials.
#[derive(Debug, Clone, Default)]
/// AuthInfo
pub struct AuthInfo {
    /// The authenticated user's identifier (sub claim, username, key id, etc.).
    pub subject: String,
    /// The user's email address, if available.
    pub email: Option<String>,
    /// The user's role, if any.
    pub role: Option<String>,
    /// The raw JWT token string, if this is a JWT authentication.
    /// Used by the token revocation endpoint to revoke the current token.
    pub token: Option<String>,
}

impl AuthInfo {
    /// Create an anonymous `AuthInfo` with an empty subject.
    #[must_use]
    /// anonymous
    pub fn anonymous() -> Self {
        Self {
            subject: String::new(),
            email: None,
            role: None,
            token: None,
        }
    }
}

/// Extractor for required authentication.
///
/// Returns `AuthInfo` if the user is authenticated, or returns a 401 Unauthorized
/// response if authentication is missing or invalid.
#[derive(Debug)]
/// AuthInfo
pub struct RequireAuth(pub AuthInfo);

/// Extractor for optional authentication.
///
/// Returns `Option<AuthInfo>` - `Some(AuthInfo)` if authenticated, or `None`
/// if authentication is not present or invalid.
#[derive(Debug)]
/// Option
pub struct OptionalAuth(pub Option<AuthInfo>);

impl<S> FromRequestParts<S> for RequireAuth
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth_info = parts.extensions.get::<AuthInfo>().cloned();

        match auth_info {
            Some(info) => Ok(RequireAuth(info)),
            None => Err((
                StatusCode::UNAUTHORIZED,
                "Missing authentication".to_string(),
            )),
        }
    }
}

impl<S> FromRequestParts<S> for OptionalAuth
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth_info = parts.extensions.get::<AuthInfo>().cloned();
        Ok(OptionalAuth(auth_info))
    }
}

/// Request context for dynamic parameter interpolation.
///
/// Populated by the auth middleware and made available to handlers.
#[derive(Debug, Clone)]
/// RequestContext
pub struct RequestContext {
    /// The unique identifier of the authenticated user, if any.
    pub user_id: Option<String>,
    /// The email address of the authenticated user, if any.
    pub user_email: Option<String>,
    /// The role assigned to the authenticated user.
    pub user_role: Option<String>,
    /// A map of relevant request headers.
    pub headers: std::collections::HashMap<String, String>,
    /// The HTTP method used for the request.
    pub method: String,
    /// The requested path.
    pub path: String,
    /// A map of query parameters.
    pub raw_query_params: std::collections::HashMap<String, String>,
}

impl Default for RequestContext {
    /// Returns a request context with all fields set to their default empty values.
    fn default() -> Self {
        Self::new()
    }
}

/// User information used to construct a `RequestContext` with identity.
///
/// This is a lightweight type used primarily in tests and internal helpers
/// to populate the user-specific fields of a `RequestContext`.
#[derive(Debug, Clone)]
/// UserInfo
pub struct UserInfo {
    /// The unique identifier of the user.
    pub id: String,
    /// The role assigned to the user, if any.
    pub role: Option<String>,
}

impl RequestContext {
    /// Creates a new, empty `RequestContext`.
    #[must_use]
    /// new
    pub fn new() -> Self {
        Self {
            user_id: None,
            user_email: None,
            user_role: None,
            headers: std::collections::HashMap::new(),
            method: String::new(),
            path: String::new(),
            raw_query_params: std::collections::HashMap::new(),
        }
    }

    /// Creates a `RequestContext` pre-populated with the given user information.
    #[must_use]
    /// new_with_user
    pub fn new_with_user(user: UserInfo) -> Self {
        Self {
            user_id: Some(user.id),
            user_email: None,
            user_role: user.role,
            headers: std::collections::HashMap::new(),
            method: String::new(),
            path: String::new(),
            raw_query_params: std::collections::HashMap::new(),
        }
    }
}

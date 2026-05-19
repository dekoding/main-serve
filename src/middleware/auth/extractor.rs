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
pub struct AuthInfo {
    /// The authenticated user's identifier (sub claim, username, key id, etc.).
    pub subject: String,
    /// The user's role, if any.
    pub role: Option<String>,
}

/// Extractor for required authentication.
///
/// Returns `AuthInfo` if the user is authenticated, or returns a 401 Unauthorized
/// response if authentication is missing or invalid.
///
/// # Examples
///
/// ```rust
/// async fn protected_handler(auth_info: RequireAuth) -> String {
///     format!("Hello, {}!", auth_info.0.subject)
/// }
/// ```
#[derive(Debug)]
pub struct RequireAuth(pub AuthInfo);

/// Extractor for optional authentication.
///
/// Returns `Option<AuthInfo>` - `Some(AuthInfo)` if authenticated, or `None`
/// if authentication is not present or invalid.
///
/// # Examples
///
/// ```rust
/// async fn optional_handler(auth_info: OptionalAuth) -> String {
///     match auth_info.0 {
///         Some(info) => format!("Hello, {}!", info.subject),
///         None => "Hello, guest!".to_string(),
///     }
/// }
/// ```
#[derive(Debug)]
pub struct OptionalAuth(pub Option<AuthInfo>);

impl<S> FromRequestParts<S> for RequireAuth
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(_parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // This will be implemented in Phase 2 when the middleware populates extensions
        unimplemented!("RequireAuth extractor requires auth middleware to be in place")
    }
}

impl<S> FromRequestParts<S> for OptionalAuth
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(_parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // This will be implemented in Phase 2 when the middleware populates extensions
        unimplemented!("OptionalAuth extractor requires auth middleware to be in place")
    }
}

/// Request context for dynamic parameter interpolation.
///
/// Populated by the auth middleware and made available to handlers.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// The unique identifier of the authenticated user, if any.
    pub user_id: Option<String>,
    /// The role assigned to the authenticated user.
    pub user_role: Option<String>,
    /// A map of relevant request headers.
    pub headers: std::collections::HashMap<String, String>,
    /// The HTTP method used for the request.
    pub method: String,
    /// The requested path.
    pub path: String,
    /// A map of query parameters.
    pub query_params: std::collections::HashMap<String, String>,
}

impl Default for RequestContext {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestContext {
    /// Creates a new, empty `RequestContext`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            user_id: None,
            user_role: None,
            headers: std::collections::HashMap::new(),
            method: String::new(),
            path: String::new(),
            query_params: std::collections::HashMap::new(),
        }
    }
}

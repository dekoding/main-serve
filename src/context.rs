/// Represents the context extracted from an HTTP request,
/// used for dynamic parameter interpolation in SQL queries.
#[derive(Debug, Clone, Default)]
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

/// Methods for `RequestContext`.
impl RequestContext {
    /// Creates a new, empty `RequestContext`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

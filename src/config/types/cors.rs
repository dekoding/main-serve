//! CORS policy configuration types.
use serde::Deserialize;

/// CORS policy configuration (global or per-endpoint override).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CorsConfig {
    /// Allowed origins (`"*"` for all).
    pub allowed_origins: Vec<String>,
    /// Allowed HTTP methods.
    pub allowed_methods: Vec<String>,
    /// Allowed request headers (`"*"` for all).
    pub allowed_headers: Vec<String>,
    /// Whether to include `Access-Control-Allow-Credentials`.
    pub allow_credentials: bool,
    /// Preflight cache duration in seconds.
    pub max_age: u64,
}

impl Default for CorsConfig {
    /// Returns a CORS configuration with default allowed methods and no allowed origins.
    fn default() -> Self {
        Self {
            allowed_origins: vec![],
            allowed_methods: vec![
                "GET".to_string(),
                "POST".to_string(),
                "PUT".to_string(),
                "PATCH".to_string(),
                "DELETE".to_string(),
                "OPTIONS".to_string(),
                "HEAD".to_string(),
            ],
            allowed_headers: vec![],
            allow_credentials: false,
            max_age: 86400,
        }
    }
}

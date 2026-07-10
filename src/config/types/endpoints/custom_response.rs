/// Custom/static response configuration types.
use std::collections::HashMap;

use serde::Deserialize;

/// Custom/static response configuration for an endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// CustomResponseConfig
pub struct CustomResponseConfig {
    /// HTTP status code.
    pub status: u16,
    /// Content-Type header value.
    pub content_type: String,
    /// Response body.
    pub body: String,
    /// Additional response headers.
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

impl Default for CustomResponseConfig {
    /// item
    fn default() -> Self {
        Self {
            status: 200,
            content_type: "application/json".to_string(),
            body: String::new(),
            headers: HashMap::new(),
        }
    }
}

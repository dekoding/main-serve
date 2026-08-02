//! Reverse proxy action configuration types.
use std::collections::HashMap;

use serde::Deserialize;

/// Proxy-specific configuration for an endpoint.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `ProxyConfig`
pub struct ProxyConfig {
    /// Upstream base URL (e.g. `"http://backend:3000"`).
    pub upstream: String,
    /// Optional path rewrite rules.
    pub path_rewrite: Option<PathRewriteConfig>,
    /// Extra headers to add to the proxied request.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Timeout settings.
    pub timeouts: ProxyTimeouts,
    /// Maximum upstream response body size in bytes (0 = unlimited).
    #[serde(default = "super::common::default_max_response_size")]
    pub max_response_size: u64,
}

/// Path rewrite rules for proxied requests.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `PathRewriteConfig`
pub struct PathRewriteConfig {
    /// Prefix to strip from the incoming path.
    pub strip_prefix: String,
    /// Prefix to prepend after stripping.
    pub add_prefix: String,
}

/// Timeout settings for proxied requests (all in seconds).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `ProxyTimeouts`
pub struct ProxyTimeouts {
    /// TCP connect timeout in seconds.
    pub connect: u64,
    /// Response read timeout in seconds.
    pub read: u64,
    /// Total request timeout in seconds.
    pub total: u64,
}

impl Default for ProxyTimeouts {
    /// Returns proxy timeout defaults with 5-second connect, 30-second read, and 60-second total.
    fn default() -> Self {
        Self {
            connect: 5,
            read: 30,
            total: 60,
        }
    }
}

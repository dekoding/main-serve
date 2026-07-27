use serde::Deserialize;

/// Rate limiting configuration (global or per-endpoint override).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `RateLimitConfig`
pub struct RateLimitConfig {
    /// Whether rate limiting is active.
    pub enabled: bool,
    /// Maximum number of requests allowed within the time window.
    pub max_requests: u64,
    /// Sliding window duration in seconds.
    pub window_seconds: u64,
    /// How to identify the client for rate-limit tracking.
    pub key_strategy: RateLimitKeyStrategy,
    /// Header name to use as the rate-limit key (when `key_strategy` is `Header`).
    pub key_header: String,
    /// Number of tracked client entries before triggering cleanup of expired entries.
    #[serde(default = "default_cleanup_threshold")]
    pub cleanup_threshold: usize,
}

/// Returns the default entry cleanup threshold (10,000) for rate-limit tracking.
const fn default_cleanup_threshold() -> usize {
    10_000
}

impl Default for RateLimitConfig {
    /// Returns a `RateLimitConfig` with rate limiting disabled by default.
    fn default() -> Self {
        Self {
            enabled: false,
            max_requests: 100,
            window_seconds: 60,
            key_strategy: RateLimitKeyStrategy::Ip,
            key_header: String::new(),
            cleanup_threshold: default_cleanup_threshold(),
        }
    }
}

/// How to extract the rate-limit key from a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
/// `RateLimitKeyStrategy`
pub enum RateLimitKeyStrategy {
    /// Use the client's remote IP address as the key.
    Ip,
    /// Use the value of a configurable request header as the key.
    Header,
    /// Use a bearer token (e.g. JWT) as the key.
    Token,
}

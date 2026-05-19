/// Rate limiting middleware.
///
/// Implements a sliding-window rate limiter keyed by IP address, header value,
/// or token. Uses an in-memory `HashMap` behind `Arc<Mutex<>>` for tracking.
///
/// This is applied as a per-request check inside handler closures rather than
/// as a tower Layer, since each endpoint can have its own rate limit config.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::{http::{HeaderMap, Request}, body::Body, middleware::Next, response::Response, extract::Extension};
use tokio::sync::Mutex;

use crate::config::types::{RateLimitConfig, RateLimitKeyStrategy};
use crate::error::AppError;

/// Entry tracking request counts within a time window.
#[derive(Debug, Clone)]
struct RateLimitEntry {
    count: u64,
    window_start: Instant,
}

/// Shared rate limiter state - a map of key -> (count, `window_start`).
#[derive(Debug, Clone)]
pub struct RateLimiter {
    entries: Arc<Mutex<HashMap<String, RateLimitEntry>>>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    /// Create a new empty rate limiter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Check if a request is allowed under the given rate limit config.
    ///
    /// Returns `Ok(())` if allowed, `Err(AppError::RateLimited)` if not.
    /// Also prunes stale entries (older than twice the window duration)
    /// when the entry count exceeds the configured cleanup threshold.
    ///
    /// # Errors
    ///
    /// Returns `AppError::RateLimited` when the request exceeds the configured
    /// maximum requests within the time window.
    pub async fn check_rate_limit(
        &self,
        config: &RateLimitConfig,
        headers: &HeaderMap,
        remote_addr: Option<SocketAddr>,
    ) -> Result<(), AppError> {
        if !config.enabled {
            return Ok(());
        }

        let key = extract_key(config, headers, remote_addr);
        let now = Instant::now();
        let window = std::time::Duration::from_secs(config.window_seconds);

        let mut entries = self.entries.lock().await;

        // Periodically prune expired entries to prevent unbounded growth.
        // Run cleanup when the map exceeds a reasonable threshold.
        if entries.len() > config.cleanup_threshold {
            let max_age = window * 2;
            entries.retain(|_, entry| now.duration_since(entry.window_start) < max_age);
        }

        let entry = entries.entry(key).or_insert_with(|| RateLimitEntry {
            count: 0,
            window_start: now,
        });

        // Reset window if expired.
        if now.duration_since(entry.window_start) >= window {
            entry.count = 0;
            entry.window_start = now;
        }

        entry.count += 1;

        if entry.count > config.max_requests {
            Err(AppError::RateLimited)
        } else {
            Ok(())
        }
    }
}

/// Extract the rate-limit key from the request based on the configured strategy.
fn extract_key(
    config: &RateLimitConfig,
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> String {
    match config.key_strategy {
        RateLimitKeyStrategy::Ip => {
            remote_addr.map_or_else(|| "unknown".to_string(), |addr| addr.ip().to_string())
        }
        RateLimitKeyStrategy::Header => headers
            .get(&config.key_header)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string(),
        RateLimitKeyStrategy::Token => headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
    .to_string(),
    }
}

/// Axum middleware wrapper for rate limiting.
///
/// Runs after auth middleware so it can use auth info (token) for rate limiting.
pub async fn rate_limit_middleware(
    state: axum::extract::State<crate::server::state::AppState>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let remote_addr = req
        .extensions()
        .get::<Extension<SocketAddr>>()
        .map(|ext| ext.0);
    
    let _path = req.uri().path().to_string();
    let endpoint_config = req
        .extensions()
        .get::<crate::config::types::EndpointConfig>()
        .cloned();
    
    let config = state.0.config.read().await;
    let rl_config = endpoint_config
        .and_then(|e| e.rate_limit)
        .unwrap_or(config.rate_limit.clone());
    drop(config);
    
    let headers = req.headers().clone();
    state.rate_limiter.check_rate_limit(&rl_config, &headers, remote_addr).await?;
    
    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(max_requests: u64) -> RateLimitConfig {
        RateLimitConfig {
            enabled: true,
            max_requests,
            window_seconds: 60,
            key_strategy: RateLimitKeyStrategy::Ip,
            key_header: String::new(),
            cleanup_threshold: 10_000,
        }
    }

    #[tokio::test]
    async fn test_rate_limit_allows_under_limit() {
        let limiter = RateLimiter::new();
        let config = test_config(5);
        let addr = Some("127.0.0.1:1234".parse().unwrap());

        for _ in 0..5 {
            assert!(
                limiter
                    .check_rate_limit(&config, &HeaderMap::new(), addr)
                    .await
                    .is_ok()
            );
        }
        // The 6th request must be rejected.
        assert!(
            limiter
                .check_rate_limit(&config, &HeaderMap::new(), addr)
                .await
                .is_err(),
            "Request exceeding the limit should be rejected"
        );
    }

    #[tokio::test]
    async fn test_rate_limit_blocks_over_limit() {
        let limiter = RateLimiter::new();
        let config = test_config(3);
        let addr = Some("127.0.0.1:1234".parse().unwrap());

        for _ in 0..3 {
            limiter
                .check_rate_limit(&config, &HeaderMap::new(), addr)
                .await
                .unwrap();
        }

        let result = limiter
            .check_rate_limit(&config, &HeaderMap::new(), addr)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_rate_limit_different_keys() {
        let limiter = RateLimiter::new();
        let config = test_config(1);

        let addr1 = Some("10.0.0.1:1234".parse().unwrap());
        let addr2 = Some("10.0.0.2:1234".parse().unwrap());

        limiter
            .check_rate_limit(&config, &HeaderMap::new(), addr1)
            .await
            .unwrap();
        // addr1 is now rate-limited.
        assert!(
            limiter
                .check_rate_limit(&config, &HeaderMap::new(), addr1)
                .await
                .is_err()
        );
        // addr2 should still be allowed.
        assert!(
            limiter
                .check_rate_limit(&config, &HeaderMap::new(), addr2)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_rate_limit_disabled() {
        let limiter = RateLimiter::new();
        let mut config = test_config(0);
        config.enabled = false;

        let addr = Some("127.0.0.1:1234".parse().unwrap());
        // Should always pass when disabled.
        assert!(
            limiter
                .check_rate_limit(&config, &HeaderMap::new(), addr)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_rate_limit_header_key() {
        let limiter = RateLimiter::new();
        let config = RateLimitConfig {
            enabled: true,
            max_requests: 1,
            window_seconds: 60,
            key_strategy: RateLimitKeyStrategy::Header,
            key_header: "X-Client-Id".to_string(),
            cleanup_threshold: 10_000,
        };

        let mut headers = HeaderMap::new();
        headers.insert("X-Client-Id", "client-a".parse().unwrap());

        limiter
            .check_rate_limit(&config, &headers, None)
            .await
            .unwrap();
        assert!(
            limiter
                .check_rate_limit(&config, &headers, None)
                .await
                .is_err()
        );
    }
}

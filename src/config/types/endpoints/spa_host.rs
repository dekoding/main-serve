//! SPA host action configuration types.
use serde::Deserialize;

use crate::config::types::DEFAULT_CACHE_MAX_AGE;

use super::common::{default_cache_max_age, default_fallback_status, default_index, default_true};
use super::static_files::CacheRuleConfig;

/// SPA hosting endpoint configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `SpaHostConfig`
pub struct SpaHostConfig {
    /// Named store to use (must match a key in `stores`).
    pub storage: String,
    /// Index file for SPA fallback.
    #[serde(default = "default_index")]
    pub index: String,
    /// Default Cache-Control max-age in seconds.
    #[serde(default = "default_cache_max_age")]
    pub cache_max_age: u64,
    /// Per-extension Cache-Control rules.
    #[serde(default)]
    pub cache_rules: Vec<CacheRuleConfig>,
    /// `ETag` generation for cache validation.
    #[serde(default = "default_true")]
    pub etag: bool,
    /// Support for HEAD requests.
    #[serde(default = "default_true")]
    pub head_support: bool,
    /// HTTP status code for SPA fallback responses (non-existent paths).
    #[serde(default = "default_fallback_status")]
    pub fallback_status: u16,
}

impl Default for SpaHostConfig {
    /// Returns an SPA host configuration with default index page, cache, and fallback status 200.
    fn default() -> Self {
        Self {
            storage: String::new(),
            index: "index.html".to_string(),
            cache_max_age: DEFAULT_CACHE_MAX_AGE,
            cache_rules: Vec::new(),
            etag: true,
            head_support: true,
            fallback_status: 200,
        }
    }
}

//! Shared prefix and wildcard pattern matching for endpoint config lookups.
//!
//! Used by both `AppState` and the route handlers to find endpoint
//! configurations when the exact path doesn't match but a prefix or
//! wildcard pattern does.

use std::collections::HashMap;
use std::hash::BuildHasher;

use tokio::sync::RwLockReadGuard;

use crate::config::types::EndpointConfig;

/// Walk up the path segments looking for prefix matches.
///
/// For path `/api/users/42`, checks in order:
/// `/api/users/42`, `/api/users`, `/api` (stop before empty).
///
/// Returns the first endpoint whose path is a prefix of `path` and
/// whose `endpoint_check` closure returns `true`.
pub fn find_prefix_match<S: BuildHasher>(
    configs: &RwLockReadGuard<'_, HashMap<String, EndpointConfig, S>>,
    path: &str,
    endpoint_check: impl Fn(&EndpointConfig) -> bool,
) -> Option<EndpointConfig> {
    let mut current = path;
    while !current.is_empty() {
        if let Some(endpoint) = configs.get(current)
            && endpoint_check(endpoint)
        {
            return Some(endpoint.clone());
        }
        if let Some(pos) = current.rfind('/') {
            current = &current[..pos];
        } else {
            break;
        }
    }
    None
}

/// Check for wildcard pattern matches (e.g. `/app/{*rest}` matches `/app/foo/bar`).
///
/// For each stored path containing `{`, extracts the base path before `{` and
/// checks if `path` starts with it. Returns the first matching endpoint
/// whose `endpoint_check` closure returns `true`.
pub fn find_wildcard_match<S: BuildHasher>(
    configs: &HashMap<String, EndpointConfig, S>,
    path: &str,
    endpoint_check: impl Fn(&EndpointConfig) -> bool,
) -> Option<EndpointConfig> {
    for (stored_path, endpoint) in configs {
        if let Some(pattern_base) = stored_path
            .split_once('{')
            .map(|(base, _)| base.trim_end_matches('/'))
            && (path == pattern_base || path.starts_with(&(pattern_base.to_string() + "/")))
            && endpoint_check(endpoint)
        {
            return Some(endpoint.clone());
        }
    }
    None
}

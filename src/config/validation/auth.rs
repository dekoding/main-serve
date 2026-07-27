use std::collections::HashMap;

use crate::config::types::{AppConfig, RoleHierarchy};

/// Validate auth references in endpoints point to configured providers.
#[allow(clippy::too_many_lines)] // single-pass validation across multiple auth provider configurations
pub fn validate_auth(config: &AppConfig, errors: &mut Vec<String>) {
    let valid_providers: Vec<&str> = {
        let mut v = vec!["none"];
        if config.auth.jwt.is_some() {
            v.push("jwt");
        }
        if config.auth.api_key.is_some() {
            v.push("api_key");
        }
        if config.auth.basic.is_some() {
            v.push("basic");
        }
        if config.auth.oauth2.is_some() {
            v.push("oauth2");
        }
        v
    };

    for (i, ep) in config.endpoints.iter().enumerate() {
        if !valid_providers.contains(&ep.auth.as_str()) {
            errors.push(format!(
                "endpoints[{}] ({}): auth '{}' is not a configured auth provider (available: {:?})",
                i, ep.path, ep.auth, valid_providers
            ));
        }
    }

    // JWT secret must be set if JWT is configured
    if let Some(ref jwt) = config.auth.jwt
        && jwt.secret.is_empty()
    {
        errors.push("auth.jwt.secret must not be empty".to_string());
    }

    // JWT revocation requires JWT to be configured (already implied but explicit)
    if let Some(ref jwt) = config.auth.jwt
        && let Some(ref revocation) = jwt.revocation
    {
        match revocation.store {
            crate::config::types::RevocationStoreType::InMemory => {}
            crate::config::types::RevocationStoreType::Database => {
                if revocation.db_table.as_deref().is_none_or(str::is_empty) {
                    errors.push(
                        "auth.jwt.revocation.db_table must not be empty when store is database"
                            .to_string(),
                    );
                }
                if let Some(interval) = revocation.cleanup_interval_secs
                    && interval == 0
                {
                    errors
                        .push("auth.jwt.revocation.cleanup_interval_secs must be > 0".to_string());
                }
            }
        }
    }

    // API keys must have at least one key if configured
    if let Some(ref api_key) = config.auth.api_key
        && api_key.keys.is_empty()
    {
        errors.push("auth.api_key.keys must contain at least one key".to_string());
    }

    // OAuth2 code flow requires JWT to be configured (for minting tokens after login)
    if let Some(ref oauth2) = config.auth.oauth2
        && !oauth2.authorization_url.is_empty()
    {
        if config.auth.jwt.is_none() {
            errors.push(
                "auth.jwt must be configured when OAuth2 code flow is enabled \
                 (authorization_url is set) because a JWT is minted after login"
                    .to_string(),
            );
        }
        if oauth2.token_url.is_empty() {
            errors.push(
                "auth.oauth2.token_url must not be empty when authorization_url is set".to_string(),
            );
        }
        if oauth2.client_id.is_empty() {
            errors.push(
                "auth.oauth2.client_id must not be empty when authorization_url is set".to_string(),
            );
        }
        if oauth2.redirect_url.is_empty() {
            errors.push(
                "auth.oauth2.redirect_url must not be empty when authorization_url is set"
                    .to_string(),
            );
        }
        // OAuth2 role mapping validation
        if let Some(ref mapping) = oauth2.role_mapping {
            if mapping.role_map.is_empty() {
                errors.push(
                    "auth.oauth2.role_mapping.role_map must not be empty when role_mapping is configured"
                        .to_string(),
                );
            }
            if mapping.default_role.is_empty() {
                errors.push("auth.oauth2.role_mapping.default_role must not be empty".to_string());
            }
        }
    }

    // Registration validation
    if let Some(ref register) = config.auth.register
        && register.enabled
    {
        if register.table.is_empty() {
            errors.push(
                "auth.register.table must not be empty when registration is enabled".to_string(),
            );
        }
        if register.database.is_empty() {
            errors.push(
                "auth.register.database must not be empty when registration is enabled".to_string(),
            );
        }
        if register.default_role.is_empty() {
            errors.push("auth.register.default_role must not be empty".to_string());
        }
    }
}

/// Validate role hierarchy configuration for structural correctness.
///
/// Checks for self-references and cycles (direct and transitive) in the
/// role hierarchy DAG. A cycle would make transitive role resolution
/// ambiguous.
pub fn validate_role_hierarchy(role_hierarchy: Option<&RoleHierarchy>, errors: &mut Vec<String>) {
    let Some(hierarchy) = role_hierarchy else {
        return;
    };

    let roles = &hierarchy.roles;

    // Check for self-loops (a role listing itself as a parent).
    for (role, parents) in roles {
        if parents.contains(role) {
            errors.push(format!(
                "role_hierarchy.{role}: role cannot list itself as a parent"
            ));
        }
    }

    // DFS-based cycle detection on the parent graph.
    // State: 0 = unvisited, 1 = visiting (in current path), 2 = visited.
    let mut state: HashMap<String, u8> = HashMap::new();
    for role in roles.keys() {
        state.insert(role.clone(), 0);
    }

    for role in roles.keys() {
        // SAFETY: The key was just inserted in the loop above.
        if state[role] == 0
            && let Some(cycle_path) = dfs_detect_cycle(role, roles, &mut state)
        {
            errors.push(format!("role_hierarchy: cycle detected: {cycle_path}"));
        }
    }
}

/// Perform a DFS from `role` through parent edges looking for cycles.
///
/// Returns `Some(path)` describing the cycle found, or `None` if no cycle
/// exists through this node.
fn dfs_detect_cycle(
    role: &str,
    roles: &HashMap<String, Vec<String>>,
    state: &mut HashMap<String, u8>,
) -> Option<String> {
    state.insert(role.to_string(), 1); // visiting

    if let Some(parents) = roles.get(role) {
        for parent in parents {
            let parent_state = state.get(parent).copied().unwrap_or(0);
            match parent_state {
                0 => {
                    // Unvisited - recurse
                    if let Some(cycle) = dfs_detect_cycle(parent, roles, state) {
                        return Some(format!("{role} -> {parent} -> {cycle}"));
                    }
                }
                1 => {
                    // Still visiting - found a cycle
                    return Some(format!("{role} -> {parent}"));
                }
                _ => {
                    // Already fully processed or unknown - no cycle through this path
                }
            }
        }
    }

    state.insert(role.to_string(), 2); // visited
    None
}

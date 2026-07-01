use std::collections::HashMap;

use super::types::{AppConfig, EndpointAction, RoleHierarchy, StoreBackend, StoreConfig};

/// Minimum valid HTTP status code.
const HTTP_STATUS_MIN: u16 = 100;
/// Maximum valid HTTP status code.
const HTTP_STATUS_MAX: u16 = 599;
use crate::error::AppError;

/// Characters allowed in SQL expressions from config (join ON clauses,
/// computed field expressions, where clauses). This rejects semicolons,
/// comments, backticks, command substitution, newlines, and other dangerous
/// SQL metacharacters while still allowing typical expressions like
/// `table.col = other.col` or `COUNT(*)`, and interpolation syntax like
/// `${request.user.id}`.
fn is_safe_sql_fragment(s: &str) -> bool {
    !s.is_empty()
        && !s.contains(';')
        && !s.contains("--")
        && !s.contains("/*")
        && !s.contains('`')
        && !s.contains("$(")
        && !s.contains('\n')
        && !s.contains('\r')
}

/// Validate a fully parsed `AppConfig` for semantic correctness.
///
/// Returns `Ok(())` if valid, or `Err(AppError::Validation(...))` with a
/// combined list of all problems found.
///
/// # Errors
///
/// Returns `AppError::Validation` containing all validation errors joined
/// into a single message.
pub fn validate_config(config: &AppConfig) -> Result<(), AppError> {
    let mut errors: Vec<String> = Vec::new();

    validate_server(&config.server, &mut errors);
    validate_databases(config, &mut errors);
    validate_stores(&config.stores, &mut errors);
    validate_tables(config, &mut errors);
    validate_endpoints(config, &mut errors);
    validate_auth(config, &mut errors);
    validate_role_hierarchy(&config.role_hierarchy, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "Configuration validation failed:\n  - {}",
            errors.join("\n  - ")
        )))
    }
}

/// Validate server settings.
fn validate_server(server: &super::types::ServerConfig, errors: &mut Vec<String>) {
    if server.host.is_empty() {
        errors.push("server.host must not be empty".to_string());
    }
    if let Some(ref tls) = server.tls {
        if tls.cert.is_empty() {
            errors.push("server.tls.cert must not be empty when TLS is configured".to_string());
        }
        if tls.key.is_empty() {
            errors.push("server.tls.key must not be empty when TLS is configured".to_string());
        }
    }
}

/// Validate database configs have required fields.
fn validate_databases(config: &AppConfig, errors: &mut Vec<String>) {
    for (name, db) in &config.databases {
        if db.url.is_empty() {
            errors.push(format!("databases.{name}.url must not be empty"));
        }
        if db.max_connections == 0 {
            errors.push(format!("databases.{name}.max_connections must be > 0"));
        }
        if db.min_connections > db.max_connections {
            errors.push(format!(
                "databases.{name}.min_connections ({}) must be <= max_connections ({})",
                db.min_connections, db.max_connections
            ));
        }
    }
}

/// Validate table schemas reference existing databases and have valid columns.
fn validate_tables(config: &AppConfig, errors: &mut Vec<String>) {
    // Check for duplicate table name + database combinations
    let mut seen_tables = std::collections::HashSet::new();

    for table in &config.tables {
        let table_key = format!("{}.{}", table.name, table.database);
        if !seen_tables.insert(table_key.clone()) {
            errors.push(format!(
                "Duplicate table definition for '{}.{}'",
                table.name, table.database
            ));
        }
    }

    // Validate each table
    for (i, table) in config.tables.iter().enumerate() {
        let label = format!("tables[{}] ({})", i, table.name);

        // Check database reference exists
        if !config.databases.contains_key(&table.database) {
            errors.push(format!(
                "{}: references database '{}' which is not defined in databases",
                label, table.database
            ));
        }

        // Check table has at least one column
        if table.columns.is_empty() {
            errors.push(format!("{}: must have at least one column", label));
        }

        // Check column names are not empty and not duplicated
        let mut col_names = std::collections::HashSet::new();
        let mut has_pk = false;
        for col in &table.columns {
            if col.name.is_empty() {
                errors.push(format!("{}: has a column with an empty name", label));
            }
            if !col_names.insert(&col.name) {
                errors.push(format!(
                    "{}: has duplicate column name '{}'",
                    label, col.name
                ));
            }
            if col.primary_key {
                has_pk = true;
            }
        }
        if !has_pk {
            errors.push(format!(
                "{}: must have at least one primary key column",
                label
            ));
        }

        // Validate foreign key references
        for fk in &table.foreign_keys {
            if !col_names.contains(&fk.column) {
                errors.push(format!(
                    "{}: foreign_keys references column '{}' which does not exist",
                    label, fk.column
                ));
            }
            // Check referenced table exists (by name + database match)
            let table_exists = config
                .tables
                .iter()
                .any(|t| t.name == fk.references_table && t.database == table.database);
            if !table_exists {
                errors.push(format!(
                    "{}: foreign_keys references table '{}' which is not defined",
                    label, fk.references_table
                ));
            }
        }
    }
}

/// Validate endpoint configs: correct action types, valid references, etc.
fn validate_endpoints(config: &AppConfig, errors: &mut Vec<String>) {
    for (i, ep) in config.endpoints.iter().enumerate() {
        let label = format!("endpoints[{}] ({})", i, ep.path);

        if ep.path.is_empty() {
            errors.push(format!("{label}: path must not be empty"));
        }
        if ep.methods.is_empty() {
            errors.push(format!("{label}: must have at least one HTTP method"));
        }

        match ep.action {
            EndpointAction::Crud => {
                if let Some(ref crud) = ep.crud {
                    if crud.table.is_empty() {
                        errors.push(format!("{label}: crud.table must not be empty"));
                    } else if !config.tables.iter().any(|t| t.name == crud.table) {
                        errors.push(format!(
                            "{label}: crud.table '{}' is not defined in tables",
                            crud.table
                        ));
                    }
                    if crud.database.is_empty() {
                        errors.push(format!("{label}: crud.database must not be empty"));
                    } else if !config.databases.contains_key(&crud.database) {
                        errors.push(format!(
                            "{label}: crud.database '{}' is not defined in databases",
                            crud.database
                        ));
                    }
                    // Validate SQL fragments embedded in queries.
                    if let Some(ref wc) = crud.where_clause
                        && !is_safe_sql_fragment(wc)
                    {
                        errors.push(format!(
                            "{label}: crud.where_clause contains unsafe SQL characters (;, --, /*)"
                        ));
                    }
                    for (ji, join) in crud.joins.iter().enumerate() {
                        if !is_safe_sql_fragment(&join.on) {
                            errors.push(format!(
                                "{label}: crud.joins[{ji}].on contains unsafe SQL characters"
                            ));
                        }
                        if !is_safe_sql_fragment(&join.table) {
                            errors.push(format!(
                                "{label}: crud.joins[{ji}].table contains unsafe SQL characters"
                            ));
                        }
                    }
                    for (ci, cf) in crud.computed_fields.iter().enumerate() {
                        if !is_safe_sql_fragment(&cf.expression) {
                            errors.push(format!(
                                "{label}: crud.computed_fields[{ci}].expression contains unsafe SQL characters"
                            ));
                        }
                        if !is_safe_sql_fragment(&cf.name) {
                            errors.push(format!(
                                "{label}: crud.computed_fields[{ci}].name contains unsafe SQL characters"
                            ));
                        }
                    }
                } else {
                    errors.push(format!(
                        "{label}: action is 'crud' but no crud config provided"
                    ));
                }
            }
            EndpointAction::Proxy => {
                if let Some(ref proxy) = ep.proxy {
                    if proxy.upstream.is_empty() {
                        errors.push(format!("{label}: proxy.upstream must not be empty"));
                    }
                } else {
                    errors.push(format!(
                        "{label}: action is 'proxy' but no proxy config provided"
                    ));
                }
            }
            EndpointAction::StaticFiles => {
                if let Some(ref sf) = ep.static_files {
                    if sf.storage.is_empty() {
                        errors.push(format!("{label}: static_files.storage must not be empty"));
                    } else if !config.stores.contains_key(&sf.storage) {
                        errors.push(format!(
                            "{label}: static_files.storage references store '{}' which is not defined in stores",
                            sf.storage
                        ));
                    }
                } else {
                    errors.push(format!(
                        "{label}: action is 'static_files' but no static_files config provided"
                    ));
                }
            }
            EndpointAction::SpaHost => {
                if let Some(ref spa) = ep.spa_host {
                    if spa.storage.is_empty() {
                        errors.push(format!("{label}: spa_host.storage must not be empty"));
                    } else if !config.stores.contains_key(&spa.storage) {
                        errors.push(format!(
                            "{label}: spa_host.storage references store '{}' which is not defined in stores",
                            spa.storage
                        ));
                    }
                } else {
                    errors.push(format!(
                        "{label}: action is 'spa_host' but no spa_host config provided"
                    ));
                }
            }
            EndpointAction::Media => {
                if let Some(ref media) = ep.media {
                    if media.storage.is_empty() {
                        errors.push(format!("{label}: media.storage must not be empty"));
                    } else if !config.stores.contains_key(&media.storage) {
                        errors.push(format!(
                            "{label}: media.storage references store '{}' which is not defined in stores",
                            media.storage
                        ));
                    }
                    if media.table.is_empty() {
                        errors.push(format!("{label}: media.table must not be empty"));
                    } else if !config.tables.iter().any(|t| t.name == media.table) {
                        errors.push(format!(
                            "{label}: media.table '{}' is not defined in tables",
                            media.table
                        ));
                    }
                    if media.database.is_empty() {
                        errors.push(format!("{label}: media.database must not be empty"));
                    } else if !config.databases.contains_key(&media.database) {
                        errors.push(format!(
                            "{label}: media.database references database '{}' which is not defined in databases",
                            media.database
                        ));
                    }
                } else {
                    errors.push(format!(
                        "{label}: action is 'media' but no media config provided"
                    ));
                }
            }
            EndpointAction::FileStore => {
                if let Some(ref fs) = ep.file_store {
                    if fs.storage.is_empty() {
                        errors.push(format!("{label}: file_store.storage must not be empty"));
                    } else if !config.stores.contains_key(&fs.storage) {
                        errors.push(format!(
                            "{label}: file_store.storage references store '{}' which is not defined in stores",
                            fs.storage
                        ));
                    }
                    if fs.table.is_empty() {
                        errors.push(format!("{label}: file_store.table must not be empty"));
                    } else if !config.tables.iter().any(|t| t.name == fs.table) {
                        errors.push(format!(
                            "{label}: file_store.table '{}' is not defined in tables",
                            fs.table
                        ));
                    }
                    if fs.database.is_empty() {
                        errors.push(format!("{label}: file_store.database must not be empty"));
                    } else if !config.databases.contains_key(&fs.database) {
                        errors.push(format!(
                            "{label}: file_store.database references database '{}' which is not defined in databases",
                            fs.database
                        ));
                    }
                } else {
                    errors.push(format!(
                        "{label}: action is 'file_store' but no file_store config provided"
                    ));
                }
            }
            EndpointAction::CustomResponse => {
                if ep.custom_response.is_none() {
                    errors.push(format!(
                        "{label}: action is 'custom_response' but no custom_response config provided"
                    ));
                } else if let Some(cr) = ep.custom_response.as_ref()
                    && (cr.status < HTTP_STATUS_MIN || cr.status > HTTP_STATUS_MAX)
                {
                    errors.push(format!(
                        "{label}: custom_response.status must be a valid HTTP status code ({HTTP_STATUS_MIN}-{HTTP_STATUS_MAX})"
                    ));
                }
            }
        }
    }
}

/// Validate auth references in endpoints point to configured providers.
fn validate_auth(config: &AppConfig, errors: &mut Vec<String>) {
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
            super::types::RevocationStoreType::InMemory => {}
            super::types::RevocationStoreType::Database => {
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
fn validate_role_hierarchy(role_hierarchy: &Option<RoleHierarchy>, errors: &mut Vec<String>) {
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

/// Validate store configurations have required fields per backend type.
///
/// Checks that each store has the correct backend-specific config present,
/// all required fields within that config are non-empty, and no conflicting
/// backend configs are set simultaneously.
fn validate_stores(
    stores: &std::collections::HashMap<String, StoreConfig>,
    errors: &mut Vec<String>,
) {
    for (name, store) in stores {
        let label = format!("stores.{name}");

        // Count non-None backend-specific config sections.
        let mut config_count: u32 = 0;
        if store.root.is_some() {
            config_count += 1;
        }
        if store.s3.is_some() {
            config_count += 1;
        }
        if store.azure.is_some() {
            config_count += 1;
        }
        if store.gcs.is_some() {
            config_count += 1;
        }

        // Spec rule: at most one backend-specific config section.
        if config_count > 1 {
            let mut present = Vec::new();
            if store.root.is_some() {
                present.push("root");
            }
            if store.s3.is_some() {
                present.push("s3");
            }
            if store.azure.is_some() {
                present.push("azure");
            }
            if store.gcs.is_some() {
                present.push("gcs");
            }
            errors.push(format!(
                "{label}: conflicting backend config sections: {present:?} (at most one allowed)"
            ));
        }

        match store.backend {
            StoreBackend::Native => {
                if store.root.as_deref().is_none_or(str::is_empty) {
                    errors.push(format!(
                        "{label}: root directory must not be empty for native backend"
                    ));
                }
            }
            StoreBackend::Memory => {
                // Memory backend requires no additional configuration.
                if store.root.as_deref().is_some_and(|s| !s.is_empty()) {
                    errors.push(format!("{label}: root must not be set for memory backend"));
                }
            }
            StoreBackend::S3 => {
                let Some(s3) = store.s3.as_ref() else {
                    errors.push(format!(
                        "{label}: s3 config section must be present for s3 backend"
                    ));
                    continue;
                };
                if s3.region.is_empty() {
                    errors.push(format!("{label}: s3.region must not be empty"));
                }
                if s3.bucket.is_empty() {
                    errors.push(format!("{label}: s3.bucket must not be empty"));
                }
                if s3.access_key.is_empty() {
                    errors.push(format!("{label}: s3.access_key must not be empty"));
                }
                if s3.secret_key.is_empty() {
                    errors.push(format!("{label}: s3.secret_key must not be empty"));
                }
            }
            StoreBackend::Azure => {
                let Some(azure) = store.azure.as_ref() else {
                    errors.push(format!(
                        "{label}: azure config section must be present for azure backend"
                    ));
                    continue;
                };
                if azure.account_name.is_empty() {
                    errors.push(format!("{label}: azure.account_name must not be empty"));
                }
                if azure.account_key.is_empty() {
                    errors.push(format!("{label}: azure.account_key must not be empty"));
                }
                if azure.container.is_empty() {
                    errors.push(format!("{label}: azure.container must not be empty"));
                }
            }
            StoreBackend::Gcs => {
                let Some(gcs) = store.gcs.as_ref() else {
                    errors.push(format!(
                        "{label}: gcs config section must be present for gcs backend"
                    ));
                    continue;
                };
                if gcs.project_id.is_empty() {
                    errors.push(format!("{label}: gcs.project_id must not be empty"));
                }
                if gcs.credentials.is_empty() {
                    errors.push(format!("{label}: gcs.credentials must not be empty"));
                }
                if gcs.bucket.is_empty() {
                    errors.push(format!("{label}: gcs.bucket must not be empty"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::types;
    use super::*;
    use std::collections::HashMap;

    use types::RolesConfig;

    fn make_native_store(root: Option<&str>) -> StoreConfig {
        StoreConfig {
            backend: StoreBackend::Native,
            root: root.map(String::from),
            s3: None,
            azure: None,
            gcs: None,
        }
    }

    fn make_s3_store(
        region: Option<&str>,
        bucket: Option<&str>,
        access_key: Option<&str>,
        secret_key: Option<&str>,
    ) -> StoreConfig {
        StoreConfig {
            backend: StoreBackend::S3,
            root: None,
            s3: Some(types::S3StoreConfig {
                region: region.unwrap_or("").to_string(),
                bucket: bucket.unwrap_or("").to_string(),
                access_key: access_key.unwrap_or("").to_string(),
                secret_key: secret_key.unwrap_or("").to_string(),
                endpoint: None,
                force_path_style: false,
            }),
            azure: None,
            gcs: None,
        }
    }

    fn make_azure_store(
        account_name: Option<&str>,
        account_key: Option<&str>,
        container: Option<&str>,
    ) -> StoreConfig {
        StoreConfig {
            backend: StoreBackend::Azure,
            root: None,
            s3: None,
            azure: Some(types::AzureStoreConfig {
                account_name: account_name.unwrap_or("").to_string(),
                account_key: account_key.unwrap_or("").to_string(),
                container: container.unwrap_or("").to_string(),
            }),
            gcs: None,
        }
    }

    fn make_gcs_store(
        project_id: Option<&str>,
        credentials: Option<&str>,
        bucket: Option<&str>,
    ) -> StoreConfig {
        StoreConfig {
            backend: StoreBackend::Gcs,
            root: None,
            s3: None,
            azure: None,
            gcs: Some(types::GcsStoreConfig {
                project_id: project_id.unwrap_or("").to_string(),
                credentials: credentials.unwrap_or("").to_string(),
                bucket: bucket.unwrap_or("").to_string(),
            }),
        }
    }

    // Helper function to construct endpoint configs with all action variants.
    #[allow(clippy::too_many_arguments)] // make_endpoint is a test helper that constructs complete EndpointConfigs; refactoring into multiple helpers would add unnecessary boilerplate in test code
    fn make_endpoint(
        path: &str,
        method: types::HttpMethod,
        action: types::EndpointAction,
        crud: Option<types::CrudConfig>,
        proxy: Option<types::ProxyConfig>,
        static_files: Option<types::StaticFilesConfig>,
        spa_host: Option<types::SpaHostConfig>,
        media: Option<types::MediaConfig>,
        file_store: Option<types::FileStoreConfig>,
        custom_response: Option<types::CustomResponseConfig>,
    ) -> types::EndpointConfig {
        types::EndpointConfig {
            path: path.to_string(),
            methods: vec![method],
            action,
            crud,
            proxy,
            static_files,
            spa_host,
            media,
            file_store,
            custom_response,
            auth: "none".to_string(),
            roles: RolesConfig::Flat(Vec::new()),
            cors: None,
            rate_limit: None,
        }
    }

    #[test]
    fn test_validate_stores_native_no_root() {
        let mut stores = HashMap::new();
        stores.insert("my_native".to_string(), make_native_store(None));
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("root directory must not be empty"));
    }

    #[test]
    fn test_validate_stores_native_with_root() {
        let mut stores = HashMap::new();
        stores.insert(
            "my_native".to_string(),
            make_native_store(Some("/tmp/assets")),
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_stores_native_empty_root() {
        let mut stores = HashMap::new();
        stores.insert("my_native".to_string(), make_native_store(Some("")));
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("root directory must not be empty"));
    }

    #[test]
    fn test_validate_stores_s3_missing_all_fields() {
        let mut stores = HashMap::new();
        stores.insert("my_s3".to_string(), make_s3_store(None, None, None, None));
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 4);
        assert!(errors.iter().any(|e| e.contains("s3.region")));
        assert!(errors.iter().any(|e| e.contains("s3.bucket")));
        assert!(errors.iter().any(|e| e.contains("s3.access_key")));
        assert!(errors.iter().any(|e| e.contains("s3.secret_key")));
    }

    #[test]
    fn test_validate_stores_s3_partial_fields() {
        let mut stores = HashMap::new();
        stores.insert(
            "my_s3".to_string(),
            make_s3_store(Some("us-east-1"), None, None, None),
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 3);
    }

    #[test]
    fn test_validate_stores_s3_complete() {
        let mut stores = HashMap::new();
        stores.insert(
            "my_s3".to_string(),
            make_s3_store(
                Some("us-east-1"),
                Some("my-bucket"),
                Some("key"),
                Some("secret"),
            ),
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_stores_s3_with_optional_fields() {
        let mut stores = HashMap::new();
        stores.insert(
            "my_s3".to_string(),
            types::StoreConfig {
                backend: StoreBackend::S3,
                root: None,
                s3: Some(types::S3StoreConfig {
                    region: "us-east-1".to_string(),
                    bucket: "my-bucket".to_string(),
                    access_key: "key".to_string(),
                    secret_key: "secret".to_string(),
                    endpoint: Some("http://localhost:9000".to_string()),
                    force_path_style: true,
                }),
                azure: None,
                gcs: None,
            },
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_stores_azure_missing_all_fields() {
        let mut stores = HashMap::new();
        stores.insert("my_azure".to_string(), make_azure_store(None, None, None));
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 3);
        assert!(errors.iter().any(|e| e.contains("azure.account_name")));
        assert!(errors.iter().any(|e| e.contains("azure.account_key")));
        assert!(errors.iter().any(|e| e.contains("azure.container")));
    }

    #[test]
    fn test_validate_stores_azure_complete() {
        let mut stores = HashMap::new();
        stores.insert(
            "my_azure".to_string(),
            make_azure_store(Some("myaccount"), Some("mykey"), Some("mycontainer")),
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_stores_gcs_missing_all_fields() {
        let mut stores = HashMap::new();
        stores.insert("my_gcs".to_string(), make_gcs_store(None, None, None));
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 3);
        assert!(errors.iter().any(|e| e.contains("gcs.project_id")));
        assert!(errors.iter().any(|e| e.contains("gcs.credentials")));
        assert!(errors.iter().any(|e| e.contains("gcs.bucket")));
    }

    #[test]
    fn test_validate_stores_gcs_complete() {
        let mut stores = HashMap::new();
        stores.insert(
            "my_gcs".to_string(),
            make_gcs_store(
                Some("my-project"),
                Some("credentials.json"),
                Some("my-bucket"),
            ),
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_stores_conflicting_backend_configs() {
        let mut stores = HashMap::new();
        stores.insert(
            "conflict".to_string(),
            types::StoreConfig {
                backend: StoreBackend::Native,
                root: Some("/tmp".to_string()),
                s3: Some(types::S3StoreConfig {
                    region: "us-east-1".to_string(),
                    bucket: "bucket".to_string(),
                    access_key: "key".to_string(),
                    secret_key: "secret".to_string(),
                    endpoint: None,
                    force_path_style: false,
                }),
                azure: None,
                gcs: None,
            },
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("conflicting backend config sections"));
    }

    #[test]
    fn test_validate_stores_memory_with_root_errors() {
        let mut stores = HashMap::new();
        stores.insert(
            "mem".to_string(),
            types::StoreConfig {
                backend: StoreBackend::Memory,
                root: Some("/tmp".to_string()),
                s3: None,
                azure: None,
                gcs: None,
            },
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("root must not be set for memory backend"));
    }

    #[test]
    fn test_validate_stores_memory_without_root_ok() {
        let mut stores = HashMap::new();
        stores.insert(
            "mem".to_string(),
            types::StoreConfig {
                backend: StoreBackend::Memory,
                root: None,
                s3: None,
                azure: None,
                gcs: None,
            },
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_stores_multiple_stores() {
        let mut stores = HashMap::new();
        stores.insert(
            "native1".to_string(),
            make_native_store(Some("/tmp/assets1")),
        );
        stores.insert(
            "native2".to_string(),
            make_native_store(Some("/tmp/assets2")),
        );
        stores.insert(
            "s3".to_string(),
            make_s3_store(
                Some("us-west-2"),
                Some("cdn-bucket"),
                Some("ak"),
                Some("sk"),
            ),
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_stores_mixed_errors() {
        let mut stores = HashMap::new();
        stores.insert("native_bad".to_string(), make_native_store(None));
        stores.insert("s3_bad".to_string(), make_s3_store(None, None, None, None));
        stores.insert(
            "native_good".to_string(),
            make_native_store(Some("/tmp/good")),
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 5); // 1 native + 4 s3
    }

    #[test]
    fn test_validate_config_staticfiles_missing_store_ref() {
        let mut config = types::AppConfig::default();
        config.endpoints.push(make_endpoint(
            "/static",
            types::HttpMethod::Get,
            types::EndpointAction::StaticFiles,
            None,
            None,
            Some(types::StaticFilesConfig {
                storage: "nonexistent_store".to_string(),
                ..Default::default()
            }),
            None,
            None,
            None,
            None,
        ));
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("nonexistent_store"));
    }

    #[test]
    fn test_validate_config_staticfiles_valid_store_ref() {
        let mut config = types::AppConfig::default();
        config
            .stores
            .insert("assets".to_string(), make_native_store(Some("/tmp/assets")));
        config.endpoints.push(make_endpoint(
            "/static",
            types::HttpMethod::Get,
            types::EndpointAction::StaticFiles,
            None,
            None,
            Some(types::StaticFilesConfig {
                storage: "assets".to_string(),
                ..Default::default()
            }),
            None,
            None,
            None,
            None,
        ));
        let result = validate_config(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_config_spa_host_missing_store_ref() {
        let mut config = types::AppConfig::default();
        config.endpoints.push(make_endpoint(
            "/app",
            types::HttpMethod::Get,
            types::EndpointAction::SpaHost,
            None,
            None,
            None,
            Some(types::SpaHostConfig {
                storage: "missing".to_string(),
                ..Default::default()
            }),
            None,
            None,
            None,
        ));
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("missing"));
    }

    #[test]
    fn test_validate_config_media_missing_refs() {
        let mut config = types::AppConfig::default();
        config.endpoints.push(make_endpoint(
            "/media",
            types::HttpMethod::Get,
            types::EndpointAction::Media,
            None,
            None,
            None,
            None,
            Some(types::MediaConfig {
                storage: "nonexistent".to_string(),
                table: "nonexistent_table".to_string(),
                database: "nonexistent_db".to_string(),
                ..Default::default()
            }),
            None,
            None,
        ));
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("nonexistent"));
    }

    #[test]
    fn test_validate_config_filestore_missing_refs() {
        let mut config = types::AppConfig::default();
        config.endpoints.push(make_endpoint(
            "/files",
            types::HttpMethod::Get,
            types::EndpointAction::FileStore,
            None,
            None,
            None,
            None,
            None,
            Some(types::FileStoreConfig {
                storage: "nonexistent".to_string(),
                table: "nonexistent_table".to_string(),
                database: "nonexistent_db".to_string(),
                ..Default::default()
            }),
            None,
        ));
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("nonexistent"));
    }

    #[test]
    fn test_validate_jwt_revocation_in_memory() {
        let mut config = types::AppConfig::default();
        config.auth.jwt = Some(types::JwtConfig {
            secret: "test-secret".to_string(),
            algorithm: types::JwtAlgorithm::HS256,
            issuer: "test".to_string(),
            audience: String::new(),
            expiry: 3600,
            role_claim: "role".to_string(),
            revocation: Some(types::JwtRevocationConfig {
                store: types::RevocationStoreType::InMemory,
                db_table: None,
                cleanup_interval_secs: Some(3600),
            }),
        });
        let result = validate_config(&config);
        assert!(result.is_ok(), "in_memory revocation should be valid");
    }

    #[test]
    fn test_validate_jwt_revocation_database_no_table() {
        let mut config = types::AppConfig::default();
        config.auth.jwt = Some(types::JwtConfig {
            secret: "test-secret".to_string(),
            algorithm: types::JwtAlgorithm::HS256,
            issuer: "test".to_string(),
            audience: String::new(),
            expiry: 3600,
            role_claim: "role".to_string(),
            revocation: Some(types::JwtRevocationConfig {
                store: types::RevocationStoreType::Database,
                db_table: None,
                cleanup_interval_secs: Some(3600),
            }),
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("db_table must not be empty"),
            "Expected db_table error, got: {msg}"
        );
    }

    #[test]
    fn test_validate_jwt_revocation_database_zero_interval() {
        let mut config = types::AppConfig::default();
        config.auth.jwt = Some(types::JwtConfig {
            secret: "test-secret".to_string(),
            algorithm: types::JwtAlgorithm::HS256,
            issuer: "test".to_string(),
            audience: String::new(),
            expiry: 3600,
            role_claim: "role".to_string(),
            revocation: Some(types::JwtRevocationConfig {
                store: types::RevocationStoreType::Database,
                db_table: Some("token_blacklist".to_string()),
                cleanup_interval_secs: Some(0),
            }),
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("cleanup_interval_secs must be > 0"),
            "Expected cleanup_interval error, got: {msg}"
        );
    }

    #[test]
    fn test_validate_role_mapping_empty_map() {
        let mut config = types::AppConfig::default();
        config.auth.oauth2 = Some(types::OAuth2Config {
            provider: "test".to_string(),
            authorization_url: "https://idp.example.com/authorize".to_string(),
            token_url: "https://idp.example.com/token".to_string(),
            userinfo_url: "https://idp.example.com/userinfo".to_string(),
            client_id: "test-client".to_string(),
            client_secret: "test-secret".to_string(),
            scopes: vec!["openid".to_string()],
            redirect_url: "http://localhost:8080/callback".to_string(),
            success_url: "/".to_string(),
            cookie_name: "token".to_string(),
            state_ttl: 300,
            max_pending_states: 1000,
            role_mapping: Some(types::RoleMappingConfig {
                default_role: "user".to_string(),
                role_claim: "groups".to_string(),
                role_map: std::collections::HashMap::new(),
                match_mode: types::RoleMatchMode::Exact,
            }),
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("role_map must not be empty"),
            "Expected role_map error, got: {msg}"
        );
    }

    #[test]
    fn test_validate_role_mapping_empty_default_role() {
        let mut config = types::AppConfig::default();
        config.auth.oauth2 = Some(types::OAuth2Config {
            provider: "test".to_string(),
            authorization_url: "https://idp.example.com/authorize".to_string(),
            token_url: "https://idp.example.com/token".to_string(),
            userinfo_url: "https://idp.example.com/userinfo".to_string(),
            client_id: "test-client".to_string(),
            client_secret: "test-secret".to_string(),
            scopes: vec!["openid".to_string()],
            redirect_url: "http://localhost:8080/callback".to_string(),
            success_url: "/".to_string(),
            cookie_name: "token".to_string(),
            state_ttl: 300,
            max_pending_states: 1000,
            role_mapping: Some(types::RoleMappingConfig {
                default_role: String::new(),
                role_claim: "groups".to_string(),
                role_map: {
                    let mut map = std::collections::HashMap::new();
                    map.insert("admin-group".to_string(), "admin".to_string());
                    map
                },
                match_mode: types::RoleMatchMode::Exact,
            }),
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("default_role must not be empty"),
            "Expected default_role error, got: {msg}"
        );
    }

    #[test]
    fn test_validate_register_disabled() {
        let mut config = types::AppConfig::default();
        config.auth.register = Some(types::RegisterConfig {
            enabled: false,
            table: String::new(),
            database: String::new(),
            default_role: String::new(),
            password_hash: types::PasswordHashAlgorithm::Argon2id,
        });
        let result = validate_config(&config);
        assert!(
            result.is_ok(),
            "disabled registration should be valid regardless of other fields"
        );
    }

    #[test]
    fn test_validate_register_enabled_no_table() {
        let mut config = types::AppConfig::default();
        config.auth.register = Some(types::RegisterConfig {
            enabled: true,
            table: String::new(),
            database: "main".to_string(),
            default_role: "user".to_string(),
            password_hash: types::PasswordHashAlgorithm::Argon2id,
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("auth.register.table must not be empty"),
            "Expected table error, got: {msg}"
        );
    }

    #[test]
    fn test_validate_register_enabled_no_database() {
        let mut config = types::AppConfig::default();
        config.auth.register = Some(types::RegisterConfig {
            enabled: true,
            table: "users".to_string(),
            database: String::new(),
            default_role: "user".to_string(),
            password_hash: types::PasswordHashAlgorithm::Argon2id,
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("auth.register.database must not be empty"),
            "Expected database error, got: {msg}"
        );
    }

    #[test]
    fn test_validate_register_enabled_no_default_role() {
        let mut config = types::AppConfig::default();
        config.auth.register = Some(types::RegisterConfig {
            enabled: true,
            table: "users".to_string(),
            database: "main".to_string(),
            default_role: String::new(),
            password_hash: types::PasswordHashAlgorithm::Argon2id,
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("auth.register.default_role must not be empty"),
            "Expected default_role error, got: {msg}"
        );
    }

    #[test]
    fn test_validate_register_enabled_valid() {
        let mut config = types::AppConfig::default();
        config.auth.register = Some(types::RegisterConfig {
            enabled: true,
            table: "users".to_string(),
            database: "main".to_string(),
            default_role: "user".to_string(),
            password_hash: types::PasswordHashAlgorithm::Argon2id,
        });
        let result = validate_config(&config);
        assert!(result.is_ok(), "valid registration config should pass");
    }

    #[test]
    fn test_validate_register_enabled_all_errors() {
        let mut config = types::AppConfig::default();
        config.auth.register = Some(types::RegisterConfig {
            enabled: true,
            table: String::new(),
            database: String::new(),
            default_role: String::new(),
            password_hash: types::PasswordHashAlgorithm::Argon2id,
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("auth.register.table"));
        assert!(msg.contains("auth.register.database"));
        assert!(msg.contains("auth.register.default_role"));
    }
}

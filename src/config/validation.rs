/// Semantic validation rules for parsed configuration.
///
/// These rules go beyond serde's structural validation - they check referential
/// integrity, logical consistency, and completeness.
use super::types::{AppConfig, EndpointAction};
use crate::error::AppError;

/// Characters allowed in SQL expressions from config (join ON clauses,
/// computed field expressions, where clauses). This rejects semicolons,
/// comments, and other dangerous SQL metacharacters while still allowing
/// typical expressions like `table.col = other.col` or `COUNT(*)`, and
/// interpolation syntax like `${request.user.id}`.
fn is_safe_sql_fragment(s: &str) -> bool {
    !s.is_empty() && !s.contains(';') && !s.contains("--") && !s.contains("/*")
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
    validate_tables(config, &mut errors);
    validate_endpoints(config, &mut errors);
    validate_auth(config, &mut errors);

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
                    if let Some(ref db) = crud.database
                        && !config.databases.contains_key(db)
                    {
                        errors.push(format!(
                            "{label}: crud.database '{db}' is not defined in databases"
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
            EndpointAction::Static => {
                if let Some(ref sf) = ep.static_files {
                    if sf.root.is_empty() {
                        errors.push(format!("{label}: static_files.root must not be empty"));
                    }
                } else {
                    errors.push(format!(
                        "{label}: action is 'static' but no static_files config provided"
                    ));
                }
            }
            EndpointAction::CustomResponse => {
                if ep.custom_response.is_none() {
                    errors.push(format!(
                        "{label}: action is 'custom_response' but no custom_response config provided"
                    ));
                } else if let Some(cr) = ep.custom_response.as_ref()
                    && (cr.status < 100 || cr.status > 599)
                {
                    errors.push(format!(
                        "{label}: custom_response.status must be a valid HTTP status code (100-599)"
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
    }
}

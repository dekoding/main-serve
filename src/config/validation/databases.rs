/// Validate database configs have required fields.
pub fn validate_databases(config: &crate::config::types::AppConfig, errors: &mut Vec<String>) {
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

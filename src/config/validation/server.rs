//! Server configuration validation.
pub fn validate_server(server: &crate::config::types::ServerConfig, errors: &mut Vec<String>) {
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
    if server.keep_alive != 0 && server.keep_alive < 1 {
        errors.push("server.keep_alive must be 0 (disabled) or >= 1".to_string());
    }
    if server.shutdown_timeout < 1 {
        errors.push("server.shutdown_timeout must be >= 1".to_string());
    }
}

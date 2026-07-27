/// Validate server settings.
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
}

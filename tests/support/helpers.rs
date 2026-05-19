use std::io::Write;
use main_serve::config::types::JwtConfig;
use tempfile::NamedTempFile;

/// Helper: Write YAML to a temp file and load it.
pub fn load_yaml(yaml: &str) -> Result<main_serve::config::AppConfig, main_serve::error::AppError> {
    let mut f = NamedTempFile::new().expect("failed to create temp file");
    f.write_all(yaml.as_bytes()).expect("failed to write");
    main_serve::config::load_config(f.path())
}

/// Helper: Create dummy JWT.
pub fn jwt_config() -> JwtConfig {
    JwtConfig {
        secret: "test-jwt-secret-key-long-enough".to_string(),
        algorithm: main_serve::config::types::JwtAlgorithm::HS256,
        issuer: "test-issuer".to_string(),
        audience: "test-audience".to_string(),
        expiry: 3600,
        role_claim: "role".to_string(),
    }
}

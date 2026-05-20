use main_serve::config::types::JwtConfig;
use std::io::Write;
use tempfile::NamedTempFile;

/// Helper: Write YAML to a temp file and load it.
pub fn load_yaml(yaml: &str) -> Result<main_serve::config::AppConfig, main_serve::error::AppError> {
    let mut f = NamedTempFile::new().expect("failed to create temp file");
    f.write_all(yaml.as_bytes()).expect("failed to write");
    main_serve::config::load_config(f.path())
}

/// Helper: Extract JWT config from YAML.
///
/// Parses the auth.jwt section from a YAML config string and returns a JwtConfig
/// that matches the server's configuration. This ensures tokens created with
/// this config will validate correctly against endpoints using that YAML config.
pub fn jwt_config(yaml: &str) -> JwtConfig {
    let config = load_yaml(yaml).expect("failed to parse YAML for jwt_config");
    config
        .auth
        .jwt
        .expect("YAML config must have auth.jwt configured")
}

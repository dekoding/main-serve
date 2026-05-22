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

/// Helper: Read a config template file (relative to the project root's config/templates/).
pub fn read_config(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("config/templates")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()))
}

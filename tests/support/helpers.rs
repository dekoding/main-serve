use std::io::Write;
use tempfile::NamedTempFile;

/// Helper: write YAML to a temp file and load it.
pub fn load_yaml(yaml: &str) -> Result<main_serve::config::AppConfig, main_serve::error::AppError> {
    let mut f = NamedTempFile::new().expect("failed to create temp file");
    f.write_all(yaml.as_bytes()).expect("failed to write");
    main_serve::config::load_config(f.path())
}

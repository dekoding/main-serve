/// Configuration module: YAML loading, typed structs, and validation.
pub mod loader;
/// types
pub mod types;
/// validation
pub mod validation;

pub use loader::load_config;
pub use types::AppConfig;

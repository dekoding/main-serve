/// Configuration module: YAML loading, typed structs, and validation.
pub mod loader;
pub mod types;
pub mod validation;

pub use loader::load_config;
pub use types::AppConfig;

/// Configuration module: YAML loading, typed structs, and validation.
pub mod loader;
/// schema registry
pub mod schema_registry;
/// types
pub mod types;
/// validation
pub mod validation;

pub use loader::load_config;
pub use types::AppConfig;

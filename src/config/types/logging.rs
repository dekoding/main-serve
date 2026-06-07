/// Logging level and output format.
use serde::Deserialize;
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    /// Log verbosity level.
    pub level: LogLevel,
    /// Log output format.
    pub format: LogFormat,
    /// Whether to log request bodies.
    pub log_request_body: bool,
    /// Whether to log response bodies.
    pub log_response_body: bool,
    /// Maximum body size in bytes that will be logged. Larger bodies are truncated.
    #[serde(default = "default_max_body_log_size")]
    pub max_body_log_size: usize,
}

fn default_max_body_log_size() -> usize {
    16 * 1024 // 16 KiB
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            format: LogFormat::Pretty,
            log_request_body: false,
            log_response_body: false,
            max_body_log_size: default_max_body_log_size(),
        }
    }
}

/// Log verbosity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// Log output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Json,
    Pretty,
}

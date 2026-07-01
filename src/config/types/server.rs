use serde::Deserialize;
use std::fmt;

/// Default maximum HTTP request body size (10 MiB).
const DEFAULT_MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

/// Server bind address, port, TLS, and runtime settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// Bind address (e.g. `"127.0.0.1"` or `"0.0.0.0"`).
    pub host: String,
    /// TCP port to listen on.
    pub port: u16,
    /// Number of tokio worker threads (0 = auto-detect from CPU cores).
    pub workers: usize,
    /// Maximum HTTP request body size in bytes.
    pub max_body_size: usize,
    /// HTTP keep-alive timeout in seconds (0 = disabled).
    pub keep_alive: u64,
    /// Graceful shutdown timeout in seconds.
    pub shutdown_timeout: u64,
    /// Optional TLS configuration.
    pub tls: Option<TlsConfig>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            workers: 0,
            max_body_size: DEFAULT_MAX_BODY_SIZE,
            keep_alive: 75,
            shutdown_timeout: 30,
            tls: None,
        }
    }
}

/// TLS certificate and key paths.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsConfig {
    /// Path to the PEM certificate chain file.
    pub cert: String,
    /// Path to the PEM private key file.
    pub key: String,
}

impl fmt::Debug for TlsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsConfig")
            .field("cert", &self.cert)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

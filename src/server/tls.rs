/// TLS support via rustls.
///
/// Loads PEM-encoded certificate and private key files, builds a `rustls::ServerConfig`,
/// and provides a TLS acceptor for wrapping TCP connections.
use std::path::Path;
use std::sync::Arc;

use rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

use crate::config::types::TlsConfig;
use crate::error::AppError;

/// Build a `TlsAcceptor` from the TLS configuration.
///
/// Reads the PEM certificate chain and private key from disk, constructs a
/// `rustls::ServerConfig`, and returns a `TlsAcceptor` ready for use.
///
/// # Errors
///
/// Returns `AppError::ConfigurationError` if the cert or key files cannot be read or
/// contain no valid PEM items.
pub fn build_tls_acceptor(tls_config: &TlsConfig) -> Result<TlsAcceptor, AppError> {
    let server_config = build_rustls_config(tls_config)?;
    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

/// Build a `rustls::ServerConfig` from cert and key file paths.
fn build_rustls_config(tls_config: &TlsConfig) -> Result<ServerConfig, AppError> {
    let cert_path = Path::new(&tls_config.cert);
    let key_path = Path::new(&tls_config.key);

    // Read certificate chain.
    let cert_file = std::fs::File::open(cert_path).map_err(|e| {
        AppError::ConfigurationError(format!(
            "Failed to open TLS cert file '{}': {e}",
            tls_config.cert
        ))
    })?;
    let mut cert_reader = std::io::BufReader::new(cert_file);
    let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
        rustls_pemfile::certs(&mut cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::ConfigurationError(format!("Failed to parse TLS certificates: {e}")))?;

    if certs.is_empty() {
        return Err(AppError::ConfigurationError(
            "TLS cert file contains no valid certificates".to_string(),
        ));
    }

    // Read private key.
    let key_file = std::fs::File::open(key_path).map_err(|e| {
        AppError::ConfigurationError(format!(
            "Failed to open TLS key file '{}': {e}",
            tls_config.key
        ))
    })?;
    let mut key_reader = std::io::BufReader::new(key_file);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| AppError::ConfigurationError(format!("Failed to parse TLS private key: {e}")))?
        .ok_or_else(|| {
            AppError::ConfigurationError("TLS key file contains no valid private key".to_string())
        })?;

    // Build server config with explicit ring crypto provider.
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| AppError::ConfigurationError(format!("Failed to set TLS protocol versions: {e}")))?
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| AppError::ConfigurationError(format!("Failed to build TLS config: {e}")))?;

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_missing_cert_file() {
        let tls_config = TlsConfig {
            cert: "/nonexistent/cert.pem".to_string(),
            key: "/nonexistent/key.pem".to_string(),
        };
        let err = match build_tls_acceptor(&tls_config) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("Expected error for missing cert file"),
        };
        assert!(
            err.contains("cert file"),
            "Error should mention cert file: {err}"
        );
    }

    #[test]
    fn test_empty_cert_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        std::fs::write(&cert_path, "").unwrap();
        std::fs::write(&key_path, "").unwrap();

        let tls_config = TlsConfig {
            cert: cert_path.to_string_lossy().to_string(),
            key: key_path.to_string_lossy().to_string(),
        };
        let err = match build_tls_acceptor(&tls_config) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("Expected error for empty cert file"),
        };
        assert!(
            err.contains("no valid certificates"),
            "Error should mention no valid certificates: {err}"
        );
    }
}

// TLS integration tests.
//
// These tests verify:
// - TLS acceptor builds correctly from valid cert/key files
// - TLS acceptor rejects invalid cert/key combinations
// - HTTPS server accepts connections with self-signed certs
// - Server health endpoint works over TLS

use main_serve::config::types::TlsConfig;
use main_serve::server::build_tls_acceptor;
use rcgen::{CertifiedKey, generate_simple_self_signed};

/// Generate a self-signed cert and key, write them to temp files.
fn generate_self_signed_cert(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .expect("generate cert");

    let cert_pem = cert.pem();
    let key_pem = signing_key.serialize_pem();

    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");

    std::fs::write(&cert_path, cert_pem).expect("write cert");
    std::fs::write(&key_path, key_pem).expect("write key");

    (cert_path, key_path)
}

#[test]
fn test_tls_acceptor_valid_cert() {
    let dir = tempfile::TempDir::new().unwrap();
    let (cert_path, key_path) = generate_self_signed_cert(dir.path());

    let config = TlsConfig {
        cert: cert_path.to_string_lossy().to_string(),
        key: key_path.to_string_lossy().to_string(),
    };

    let result = build_tls_acceptor(&config);
    assert!(
        result.is_ok(),
        "Should build TLS acceptor from valid cert/key"
    );
}

#[test]
fn test_tls_acceptor_missing_cert() {
    let config = TlsConfig {
        cert: "/nonexistent/cert.pem".to_string(),
        key: "/nonexistent/key.pem".to_string(),
    };

    let err = match build_tls_acceptor(&config) {
        Err(e) => format!("{e}"),
        Ok(_) => panic!("Expected error for missing cert"),
    };
    assert!(err.contains("cert file"), "Error: {err}");
}

#[test]
fn test_tls_acceptor_invalid_cert_content() {
    let dir = tempfile::TempDir::new().unwrap();
    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");

    std::fs::write(&cert_path, "not a real certificate").unwrap();
    std::fs::write(&key_path, "not a real key").unwrap();

    let config = TlsConfig {
        cert: cert_path.to_string_lossy().to_string(),
        key: key_path.to_string_lossy().to_string(),
    };

    let err = match build_tls_acceptor(&config) {
        Err(e) => format!("{e}"),
        Ok(_) => panic!("Expected error for invalid cert content"),
    };
    assert!(err.contains("no valid certificates"), "Error: {err}");
}

#[test]
fn test_tls_acceptor_cert_without_key() {
    let dir = tempfile::TempDir::new().unwrap();
    let (cert_path, _key_path) = generate_self_signed_cert(dir.path());

    // Write an empty key file.
    let empty_key = dir.path().join("empty_key.pem");
    std::fs::write(&empty_key, "").unwrap();

    let config = TlsConfig {
        cert: cert_path.to_string_lossy().to_string(),
        key: empty_key.to_string_lossy().to_string(),
    };

    let err = match build_tls_acceptor(&config) {
        Err(e) => format!("{e}"),
        Ok(_) => panic!("Expected error for missing key"),
    };
    assert!(err.contains("no valid private key"), "Error: {err}");
}

#[tokio::test]
async fn test_tls_server_health_endpoint() {
    use main_serve::config::load_config;
    use main_serve::server::{AppState, build_router};
    use tokio::net::TcpListener;

    let dir = tempfile::TempDir::new().unwrap();
    let (cert_path, key_path) = generate_self_signed_cert(dir.path());

    // Write a minimal config with TLS enabled.
    let yaml = format!(
        r#"
server:
  host: "127.0.0.1"
  port: 0
  tls:
    cert: "{}"
    key: "{}"

endpoints:
  - path: "/api/info"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{{"test": true}}'
    auth: "none"
"#,
        cert_path.display(),
        key_path.display()
    );

    let config_file = dir.path().join("config.yaml");
    std::fs::write(&config_file, &yaml).unwrap();

    let config = load_config(&config_file).expect("load config");
    let tls_config = config.server.tls.clone().expect("tls config present");

    let state = AppState::new(config, config_file.clone(), "test-token".to_string())
        .await
        .unwrap();
    let config_guard = state.config.read().await;
    let app = build_router(&config_guard, state.clone()).await;
    drop(config_guard);

    let acceptor = build_tls_acceptor(&tls_config).expect("build acceptor");

    // Bind to a random port.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Spawn TLS server.
    let server_handle = tokio::spawn(async move {
        use hyper::body::Incoming;
        use hyper_util::rt::{TokioExecutor, TokioIo};
        use hyper_util::server::conn::auto::Builder;
        use tower::Service;

        let builder = Builder::new(TokioExecutor::new());

        // Accept just one connection for the test.
        let (tcp_stream, _remote_addr) = listener.accept().await.unwrap();
        let tls_stream = acceptor.accept(tcp_stream).await.unwrap();
        let io = TokioIo::new(tls_stream);

        let service = hyper::service::service_fn(move |req: hyper::Request<Incoming>| {
            let mut app = app.clone();
            async move { app.call(req).await }
        });

        builder.serve_connection(io, service).await.unwrap();
    });

    // Build a reqwest client that trusts our self-signed cert.
    let cert_pem = std::fs::read(&cert_path).unwrap();
    let cert = reqwest::Certificate::from_pem(&cert_pem).unwrap();

    let client = reqwest::Client::builder()
        .add_root_certificate(cert)
        .connection_verbose(false)
        .pool_max_idle_per_host(0) // Don't keep connections alive.
        .build()
        .unwrap();

    let response = client
        .get(format!(
            "https://localhost:{}/_main-serve/health",
            addr.port()
        ))
        .header("connection", "close")
        .send()
        .await
        .expect("HTTPS request should succeed");

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["status"], "healthy");

    // Wait for server task to finish.
    let _ = server_handle.await;
}

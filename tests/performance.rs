/// Performance/stress tests.
///
/// These tests are NOT run in CI by default. They are designed to verify
/// server behavior under load and should be run manually when needed.
///
/// Run with: `cargo test --test performance`
mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Stress test: verify the server can handle 1000 concurrent connections.
///
/// This is a stress test, not a unit/integration test. It creates a real
/// TCP listener, spawns the server, and sends 1000 concurrent HTTP requests.
/// Run manually before major deployments.
#[tokio::test]
async fn test_static_file_serving_concurrent() {
    // Test that the server can handle at least 1000 concurrent connections
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    std::fs::create_dir_all(&public_dir).unwrap();
    std::fs::write(
        public_dir.join("index.html"),
        "<html><body>Concurrent Test</body></html>",
    )
    .unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

endpoints:
  - path: "/static/*"
    methods: ["get"]
    action: "static"
    static_files:
      root: "{root}"
      index: "index.html"
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    // Bind to a real port and spawn the server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind to TCP listener");
    let server_addr = listener.local_addr().expect("Failed to get local addr");
    let server_url = format!("http://{}", server_addr);

    // Clone the app for spawning
    let app_clone = app.clone();

    // Spawn the server in the background
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app_clone)
            .await
            .expect("Server failed");
    });

    // Give the server a moment to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Spawn 1000 concurrent HTTP requests to the real server
    let mut handles = Vec::new();
    let num_requests = 1000;

    for i in 0..num_requests {
        let url = server_url.clone();
        let handle = tokio::spawn(async move {
            let client = reqwest::Client::new();
            let response = client
                .get(format!("{}/static/index.html", url))
                .send()
                .await
                .expect("Request failed");

            assert_eq!(
                response.status(),
                reqwest::StatusCode::OK,
                "Request {i} failed with status {}",
                response.status()
            );
            let body = response.text().await.expect("Failed to read body");
            assert!(
                body.contains("Concurrent Test"),
                "Request {i} body mismatch: {}",
                body
            );
        });
        handles.push(handle);
    }

    // Wait for all requests to complete
    let mut success_count = 0;
    for handle in handles {
        if handle.await.is_ok() {
            success_count += 1;
        }
    }

    // Verify we handled at least 1000 requests
    assert!(
        success_count == num_requests,
        "Expected to handle {} concurrent connections, got {}",
        num_requests,
        success_count
    );

    // Shutdown the server by killing the task
    server_handle.abort();
}

/// Simple load test: verify response time under moderate load.
///
/// Sends 100 sequential requests and measures timing.
#[tokio::test]
async fn test_static_file_load_timing() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    std::fs::create_dir_all(&public_dir).unwrap();
    std::fs::write(
        public_dir.join("index.html"),
        "<html><body>Load Test</body></html>",
    )
    .unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

endpoints:
  - path: "/static/*"
    methods: ["get"]
    action: "static"
    static_files:
      root: "{root}"
      index: "index.html"
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    let start = std::time::Instant::now();
    for i in 0..100 {
        let app_clone = app.clone();
        let req = Request::builder()
            .uri("/static/index.html")
            .body(Body::empty())
            .unwrap();
        let response = app_clone.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = response.into_body().collect().await.unwrap().to_bytes();
        // Small yield to allow other tasks to run
        if i % 10 == 0 {
            tokio::task::yield_now().await;
        }
    }
    let elapsed = start.elapsed();

    // 100 requests should complete in under 5 seconds
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "100 sequential requests took {}ms (expected < 5000ms)",
        elapsed.as_millis()
    );
}

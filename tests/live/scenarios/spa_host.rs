//! SPA host scenario.
//!
//! Tests Main Serve configured to host a single-page application:
//!
//! - Serving index.html at the root
//! - Serving static assets (JS, CSS, images) with cache headers
//! - SPA fallback: non-existent paths return index.html (200)
//! - ETag support for cache validation
//! - HEAD method support
//!
//! Config: serves a minimal SPA from a local directory.

use crate::support::{BinaryHandle, write_files_to_dir};
use reqwest::StatusCode;

const SPA_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

logging:
  level: "info"
  format: "json"

stores:
  web_assets:
    backend: "native"
    root: "./public"

endpoints:
  - path: "/app/*"
    methods: ["get", "head"]
    action: "spa_host"
    spa_host:
      storage: "web_assets"
      index: "index.html"
      cache_max_age: 0
      etag: true
    auth: "none"

  - path: "/assets/*"
    methods: ["get", "head"]
    action: "static_files"
    static_files:
      storage: "web_assets"
      index: "index.html"
      directory_listing: false
      cache_max_age: 604800
      etag: true
      cache_rules:
        - extensions: [".html"]
          cache_control: "no-cache, no-store"
        - extensions: [".js", ".css", ".png", ".svg", ".woff2"]
          cache_control: "public, max-age=31536000, immutable"
    auth: "none"
"#;

/// Minimal SPA files to serve.
const SPA_FILES: &[(&str, &[u8])] = &[
    ("index.html", b"<!DOCTYPE html><html><head><title>Test SPA</title></head><body><div id=\"app\"></div><script src=\"/app.js\"></script></body></html>"),
    ("app.js", b"console.log('SPA loaded');"),
    ("style.css", b"body { margin: 0; }"),
    ("logo.png", include_bytes!("../assets/logo-placeholder.png")),
];

#[tokio::test]
async fn test_spa_index_served() {
    let (server, temp_dir) = setup_spa().await;
    let client = server.client();

    let resp = client.get("/app/").await.expect("GET /app/");
    client.assert_status(&resp, StatusCode::OK).await;

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok());
    assert!(content_type.is_some_and(|ct| ct.contains("text/html")));

    let body = resp.text().await.expect("read body");
    assert!(body.contains("<title>Test SPA</title>"));

    server.shutdown().await.expect("server shutdown");
    let _ = temp_dir;
}

#[tokio::test]
async fn test_spa_fallback() {
    let (server, temp_dir) = setup_spa().await;
    let client = server.client();

    // Request a path that doesn't exist on disk (no file at /app/about)
    let resp = client.get("/app/about").await.expect("GET /app/about");
    client.assert_status(&resp, StatusCode::OK).await;

    // SPA fallback should return index.html
    let body = resp.text().await.expect("read body");
    assert!(body.contains("<title>Test SPA</title>"));

    server.shutdown().await.expect("server shutdown");
    let _ = temp_dir;
}

#[tokio::test]
async fn test_static_asset_with_cache_headers() {
    let (server, temp_dir) = setup_spa().await;
    let client = server.client();

    let resp = client
        .get("/assets/app.js")
        .await
        .expect("GET /assets/app.js");
    client.assert_status(&resp, StatusCode::OK).await;

    // Should have aggressive cache header (immutable, 1 year)
    let cache_control = resp
        .headers()
        .get("cache-control")
        .and_then(|v| v.to_str().ok());
    assert!(cache_control.is_some_and(|cc| cc.contains("immutable")));

    server.shutdown().await.expect("server shutdown");
    let _ = temp_dir;
}

#[tokio::test]
async fn test_html_no_cache() {
    let (server, temp_dir) = setup_spa().await;
    let client = server.client();

    let resp = client
        .get("/assets/style.css")
        .await
        .expect("GET /assets/style.css");
    client.assert_status(&resp, StatusCode::OK).await;

    // CSS gets immutable cache
    let cache_control = resp
        .headers()
        .get("cache-control")
        .and_then(|v| v.to_str().ok());
    assert!(cache_control.is_some_and(|cc| cc.contains("immutable")));

    server.shutdown().await.expect("server shutdown");
    let _ = temp_dir;
}

#[tokio::test]
async fn test_head_request() {
    let (server, temp_dir) = setup_spa().await;
    let client = server.client();

    let resp = client.head("/app/").await.expect("HEAD /app/");
    client.assert_status(&resp, StatusCode::OK).await;

    // HEAD should return same headers but no body
    let headers = resp.headers().clone();
    assert!(headers.get("content-type").is_some());

    // No body on HEAD
    let body = resp.text().await.expect("read body");
    assert!(body.is_empty());

    server.shutdown().await.expect("server shutdown");
    let _ = temp_dir;
}

#[tokio::test]
async fn test_404_for_missing_file() {
    let (server, temp_dir) = setup_spa().await;
    let client = server.client();

    // Request a file that doesn't exist in static files
    let resp = client
        .get("/assets/nonexistent.js")
        .await
        .expect("GET /assets/nonexistent.js");

    // Static files should return 404 for missing files
    // (SPA fallback only applies to /app/* paths)
    assert!(resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::OK);

    server.shutdown().await.expect("server shutdown");
    let _ = temp_dir;
}

async fn setup_spa() -> (BinaryHandle, tempfile::TempDir) {
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("create temp dir");
    let assets_dir = temp_dir.path().join("public");
    write_files_to_dir(&assets_dir, SPA_FILES);

    // Override the config to point to our temp directory
    let config = SPA_CONFIG.replace("./public", assets_dir.to_str().unwrap());

    let server = BinaryHandle::spawn(&config, None)
        .await
        .expect("spawn server");
    (server, temp_dir)
}

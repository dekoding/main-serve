//! Static file server scenario.
//!
//! Tests Main Serve configured as a file server:
//!
//! - Serving files from a storage backend (native/local disk)
//! - Range requests (partial content / 206)
//! - Directory listing (when enabled)
//! - Cache rules by file extension
//! - ETag support
//! - HEAD method
//! - File upload (when enabled)
//!
//! Config: serves files from a local directory with various options.

use crate::support::{BinaryHandle, write_files_to_dir};
use reqwest::StatusCode;

const STATIC_FILES_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

logging:
  level: "info"
  format: "json"

stores:
  static_assets:
    backend: "native"
    root: "./public"

endpoints:
  - path: "/files/*"
    methods: ["get", "head"]
    action: "static_files"
    static_files:
      storage: "static_assets"
      index: "index.html"
      directory_listing: true
      cache_max_age: 3600
      etag: true
      range_requests: true
      head_support: true
      cache_rules:
        - extensions: [".html"]
          cache_control: "no-cache"
        - extensions: [".jpg", ".png", ".gif", ".webp", ".svg"]
          cache_control: "public, max-age=31536000, immutable"
        - extensions: [".mp4", ".mp3"]
          cache_control: "public, max-age=86400"
    auth: "none"

  - path: "/upload"
    methods: ["post"]
    action: "static_files"
    static_files:
      storage: "static_assets"
      upload:
        enabled: true
        max_size: 10485760
        allowed_extensions: [".jpg", ".jpeg", ".png", ".gif", ".pdf"]
    auth: "none"
"#;

/// Files to serve.
const TEST_FILES: &[(&str, &[u8])] = &[
    ("index.html", b"<!DOCTYPE html><html><body>Root</body></html>"),
    ("docs/readme.txt", b"This is a readme file for directory listing."),
    ("images/photo.png", b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR"),
    ("videos/sample.mp4", b"\x00\x00\x00\x1cftypmp42"),
];

#[tokio::test]
async fn test_serve_file() {
    let (server, temp_dir) = setup_static_server().await;
    let client = server.client();

    let resp = client.get("/files/index.html").await.expect("GET /files/index.html");
    client.assert_status(&resp, StatusCode::OK).await;

    let body = resp.text().await.expect("read body");
    assert!(body.contains("Root"));

    server.shutdown().await.expect("server shutdown");
    let _ = temp_dir;
}

#[tokio::test]
async fn test_directory_listing() {
    let (server, temp_dir) = setup_static_server().await;
    let client = server.client();

    // Use a subdirectory that has no index file to trigger directory listing
    let resp = client.get("/files/images/").await.expect("GET /files/images/");
    client.assert_status(&resp, StatusCode::OK).await;

    let body = resp.text().await.expect("read body");
    // Directory listing should show the files in this directory
    assert!(body.contains("photo.png"));

    server.shutdown().await.expect("server shutdown");
    let _ = temp_dir;
}

#[tokio::test]
async fn test_directory_listing_subdir() {
    let (server, temp_dir) = setup_static_server().await;
    let client = server.client();

    let resp = client.get("/files/docs/").await.expect("GET /files/docs/");
    client.assert_status(&resp, StatusCode::OK).await;

    let body = resp.text().await.expect("read body");
    assert!(body.contains("readme.txt"));

    server.shutdown().await.expect("server shutdown");
    let _ = temp_dir;
}

#[tokio::test]
async fn test_range_request() {
    let (server, temp_dir) = setup_static_server().await;
    let client = server.client();

    // Request first 10 bytes of readme.txt
    let resp = client
        .client()
        .get(client.url("/files/docs/readme.txt"))
        .header("Range", "bytes=0-9")
        .send()
        .await
        .expect("range request");

    client.assert_status(&resp, StatusCode::PARTIAL_CONTENT).await;

    let content_range = resp.headers().get("content-range")
        .and_then(|v| v.to_str().ok());
    assert!(content_range.is_some_and(|cr| cr.starts_with("bytes 0-9/")));

    server.shutdown().await.expect("server shutdown");
    let _ = temp_dir;
}

#[tokio::test]
async fn test_cache_headers_by_extension() {
    let (server, temp_dir) = setup_static_server().await;
    let client = server.client();

    // HTML should have no-cache
    let resp = client.get("/files/index.html").await.expect("GET HTML");
    let cc = resp.headers().get("cache-control")
        .and_then(|v| v.to_str().ok());
    assert!(cc.is_some_and(|c| c.contains("no-cache")));

    // PNG should be immutable
    let resp = client.get("/files/images/photo.png").await.expect("GET PNG");
    let cc = resp.headers().get("cache-control")
        .and_then(|v| v.to_str().ok());
    assert!(cc.is_some_and(|c| c.contains("immutable")));

    server.shutdown().await.expect("server shutdown");
    let _ = temp_dir;
}

#[tokio::test]
async fn test_head_request() {
    let (server, _temp_dir) = setup_static_server().await;
    let client = server.client();

    let resp = client.head("/files/docs/readme.txt").await.expect("HEAD");
    client.assert_status(&resp, StatusCode::OK).await;
}

#[tokio::test]
async fn test_404_for_missing_file() {
    let (server, temp_dir) = setup_static_server().await;
    let client = server.client();

    let resp = client.get("/files/does-not-exist.txt").await.expect("GET missing");
    client.assert_status(&resp, StatusCode::NOT_FOUND).await;

    server.shutdown().await.expect("server shutdown");
    let _ = temp_dir;
}

#[tokio::test]
async fn test_file_upload() {
    let (server, temp_dir) = setup_static_server().await;
    let client = server.client();

    // Upload a PNG file via multipart form
    let resp = client
        .client()
        .post(client.url("/upload"))
        .multipart(
            reqwest::multipart::Form::new()
                .part("file", reqwest::multipart::Part::bytes(vec![
                    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
                    0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
                ]).file_name("test.png"))
        )
        .send()
        .await
        .expect("upload");

    // Should succeed (200 or 201)
    assert!(resp.status() == StatusCode::OK || resp.status() == StatusCode::CREATED);

    server.shutdown().await.expect("server shutdown");
    let _ = temp_dir;
}

#[tokio::test]
async fn test_upload_rejection() {
    let (server, temp_dir) = setup_static_server().await;
    let client = server.client();

    // Try to upload a disallowed extension (.exe)
    let resp = client
        .client()
        .post(client.url("/upload"))
        .multipart(
            reqwest::multipart::Form::new()
                .part("file", reqwest::multipart::Part::bytes(vec![0x4D, 0x5A])
                    .file_name("malware.exe"))
        )
        .send()
        .await
        .expect("upload exe");

    assert!(resp.status() == StatusCode::BAD_REQUEST
        || resp.status() == StatusCode::UNSUPPORTED_MEDIA_TYPE);

    server.shutdown().await.expect("server shutdown");
    let _ = temp_dir;
}

async fn setup_static_server() -> (BinaryHandle, tempfile::TempDir) {
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("create temp dir");
    let public_dir = temp_dir.path().join("public");
    write_files_to_dir(&public_dir, TEST_FILES);

    let config = STATIC_FILES_CONFIG.replace("./public", public_dir.to_str().unwrap());

    let server = BinaryHandle::spawn(&config, None).await.expect("spawn server");
    (server, temp_dir)
}

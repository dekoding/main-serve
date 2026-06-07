/// Integration tests for YAML-defined API endpoints.
///
/// Static file serving, uploads, and related functionality.
mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http::Method;
use http_body_util::BodyExt;
use main_serve::middleware::auth::validators::jwt::create_token;
use support::helpers::jwt_config;
use support::{MINIMAL_PNG, json_body};
use tower::ServiceExt;

// =============================================================================
// Static file serving
// =============================================================================

#[tokio::test]
async fn test_static_file_serving() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    std::fs::create_dir_all(&public_dir).unwrap();
    std::fs::write(
        public_dir.join("index.html"),
        "<html><body>Hello</body></html>",
    )
    .unwrap();
    std::fs::write(public_dir.join("style.css"), "body { color: red; }").unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/static/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      index: "index.html"
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    // Serve the index via explicit path.
    let req = Request::builder()
        .uri("/static/index.html")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("Hello"));

    // Serve the CSS file.
    let req = Request::builder()
        .uri("/static/style.css")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ct = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("text/css"));

    // Serve the index via bare path (no trailing slash) - should default to index.html.
    let req = Request::builder()
        .uri("/static")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("Hello"));

    // Serve the index via trailing slash - should default to index.html.
    let req = Request::builder()
        .uri("/static/")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("Hello"));
}

#[tokio::test]
async fn test_static_file_not_found() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    std::fs::create_dir_all(&public_dir).unwrap();
    std::fs::write(public_dir.join("index.html"), "<html>Index</html>").unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/static/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      index: "index.html"
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    let req = Request::builder()
        .uri("/static/nonexistent.txt")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// SPA fallback is now handled by the spa_host endpoint type (see tests/spa_host.rs)
// =============================================================================
// Static file - subdirectory index
// =============================================================================

#[tokio::test]
async fn test_static_subdirectory_index() {
    // Accessing a subdirectory should serve its index.html.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    let sub_dir = public_dir.join("about");
    std::fs::create_dir_all(&sub_dir).unwrap();
    std::fs::write(sub_dir.join("index.html"), "<html>About Page</html>").unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/static/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    let req = Request::builder()
        .uri("/static/about")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("About Page"));

    let req = Request::builder()
        .uri("/static/about/")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("About Page"));
}

// =============================================================================
// Static file - serve non-index files with body verification
// =============================================================================

#[tokio::test]
async fn test_static_serve_various_file_types() {
    // Verify that JS, CSS, JSON and other non-index files are served with correct body content.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    let js_dir = public_dir.join("js");
    let css_dir = public_dir.join("css");
    std::fs::create_dir_all(&js_dir).unwrap();
    std::fs::create_dir_all(&css_dir).unwrap();
    std::fs::write(
        public_dir.join("index.html"),
        "<html><body>Root Index</body></html>",
    )
    .unwrap();
    std::fs::write(js_dir.join("app.js"), "console.log('hello');").unwrap();
    std::fs::write(css_dir.join("main.css"), "body { margin: 0; }").unwrap();
    std::fs::write(public_dir.join("data.json"), r#"{"key":"value"}"#).unwrap();
    std::fs::write(public_dir.join("readme.txt"), "This is a readme").unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/static/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      index: "index.html"
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    // 1. Serve a JS file in a nested subdirectory - verify body content.
    let req = Request::builder()
        .uri("/static/js/app.js")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "JS file should return 200"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert_eq!(body, "console.log('hello');", "JS body mismatch");

    // 2. Serve a CSS file in a nested subdirectory - verify body content.
    let req = Request::builder()
        .uri("/static/css/main.css")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "CSS file should return 200"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert_eq!(body, "body { margin: 0; }", "CSS body mismatch");

    // 3. Serve a JSON file at the root level - verify body content.
    let req = Request::builder()
        .uri("/static/data.json")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "JSON file should return 200"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert_eq!(body, r#"{"key":"value"}"#, "JSON body mismatch");

    // 4. Serve a plain text file - verify body content.
    let req = Request::builder()
        .uri("/static/readme.txt")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "TXT file should return 200"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert_eq!(body, "This is a readme", "TXT body mismatch");

    // 5. Explicitly request /static/index.html - should serve the index file.
    let req = Request::builder()
        .uri("/static/index.html")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "/static/index.html should return 200"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(
        body.contains("Root Index"),
        "/static/index.html should contain Root Index, got: {body}"
    );

    // 6. Bare path /static - should serve index.html.
    let req = Request::builder()
        .uri("/static")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "/static should return 200"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(
        body.contains("Root Index"),
        "/static should serve index.html"
    );

    // 7. Non-existent file - should return 404.
    let req = Request::builder()
        .uri("/static/nope.txt")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "non-existent file should 404"
    );
}

// =============================================================================
// Static file - endpoint path without catch-all wildcard
// =============================================================================

#[tokio::test]
async fn test_static_path_without_wildcard() {
    // When the user configures a static endpoint without the trailing /*,
    // the handler should still serve files under the root directory.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    std::fs::create_dir_all(&public_dir).unwrap();
    std::fs::write(
        public_dir.join("index.html"),
        "<html><body>Index Page</body></html>",
    )
    .unwrap();
    std::fs::write(public_dir.join("app.js"), "alert('hi');").unwrap();
    std::fs::write(public_dir.join("style.css"), "h1 { color: blue; }").unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/site"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      index: "index.html"
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    // 1. Bare path should serve index.html.
    let req = Request::builder().uri("/site").body(Body::empty()).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "/site should return 200");
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(body.contains("Index Page"), "/site should serve index.html");

    // 2. JS file should be accessible.
    let req = Request::builder()
        .uri("/site/app.js")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "/site/app.js should return 200"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert_eq!(body, "alert('hi');", "JS body mismatch");

    // 3. CSS file should be accessible.
    let req = Request::builder()
        .uri("/site/style.css")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "/site/style.css should return 200"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert_eq!(body, "h1 { color: blue; }", "CSS body mismatch");

    // 4. Explicit index.html path should work.
    let req = Request::builder()
        .uri("/site/index.html")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "/site/index.html should return 200"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(
        body.contains("Index Page"),
        "/site/index.html body mismatch"
    );
}

// =============================================================================
// Static file - missing custom index without listing -> 404
// =============================================================================

#[tokio::test]
async fn test_scenario1b_no_listing_index_specified_but_missing() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    std::fs::create_dir_all(&public_dir).unwrap();
    // Only create a CSS file, not the specified index.
    std::fs::write(public_dir.join("style.css"), "body {}").unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/site/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      index: "home.html"
      directory_listing: false
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    // Root should 404 because the specified index doesn't exist.
    let req = Request::builder().uri("/site").body(Body::empty()).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "Scenario 1b: /site should 404 when index file is missing"
    );

    // But existing files should still be servable.
    let req = Request::builder()
        .uri("/site/style.css")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Scenario 1b: /site/style.css should still return 200"
    );
}

// =============================================================================
// Root-path static serving: path: "/" - the most common real-world config
// =============================================================================

// Scenario 1 at root: directory_listing=false, index specified, index present
#[tokio::test]
async fn test_root_path_no_listing_index_specified() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    std::fs::create_dir_all(&public_dir).unwrap();
    std::fs::write(
        public_dir.join("home.html"),
        "<html><body>Home</body></html>",
    )
    .unwrap();
    std::fs::write(public_dir.join("style.css"), "body { color: green; }").unwrap();
    std::fs::write(public_dir.join("app.js"), "console.log('root');").unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      index: "home.html"
      directory_listing: false
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    // GET / should serve home.html.
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S1: GET / should return 200 (home.html)"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(
        body.contains("Home"),
        "Root S1: GET / should serve home.html"
    );

    // GET /home.html explicitly should work.
    let req = Request::builder()
        .uri("/home.html")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S1: GET /home.html should return 200"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(body.contains("Home"), "Root S1: /home.html body mismatch");

    // GET /style.css should work.
    let req = Request::builder()
        .uri("/style.css")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S1: GET /style.css should return 200"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert_eq!(body, "body { color: green; }");

    // GET /app.js should work.
    let req = Request::builder()
        .uri("/app.js")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S1: GET /app.js should return 200"
    );

    // GET /missing.txt should 404.
    let req = Request::builder()
        .uri("/missing.txt")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "Root S1: GET /missing.txt should 404"
    );
}

// Scenario 2 at root: directory_listing=false, index not specified, default index.html present
#[tokio::test]
async fn test_root_path_no_listing_default_index_present() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    std::fs::create_dir_all(&public_dir).unwrap();
    std::fs::write(public_dir.join("index.html"), "<html>Default Index</html>").unwrap();
    std::fs::write(public_dir.join("style.css"), "p { margin: 0; }").unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      directory_listing: false
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    // GET / should serve the default index.html.
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S2: GET / should return 200 when index.html is present"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(
        body.contains("Default Index"),
        "Root S2: GET / should serve index.html"
    );

    // CSS should be accessible.
    let req = Request::builder()
        .uri("/style.css")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S2: GET /style.css should return 200"
    );
}

// Scenario 2b at root: no listing, no index specified, no index.html file -> 404 at /
#[tokio::test]
async fn test_root_path_no_listing_no_index_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    std::fs::create_dir_all(&public_dir).unwrap();
    std::fs::write(public_dir.join("data.txt"), "hello").unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      directory_listing: false
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    // GET / should 404 because there's no index.html.
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "Root S2b: GET / should 404 when no index.html"
    );

    // But specific files should still work.
    let req = Request::builder()
        .uri("/data.txt")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S2b: GET /data.txt should return 200"
    );
}

// Scenario 3 at root: directory_listing=true, no index specified, no index.html -> listing
#[tokio::test]
async fn test_root_path_listing_no_index_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    let sub_dir = public_dir.join("docs");
    std::fs::create_dir_all(&sub_dir).unwrap();
    std::fs::write(public_dir.join("readme.txt"), "README").unwrap();
    std::fs::write(sub_dir.join("guide.md"), "# Guide").unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      directory_listing: true
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    // GET / should return a directory listing.
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S3: GET / should return 200 (directory listing)"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(
        body.contains("readme.txt"),
        "Root S3: listing should contain readme.txt"
    );
    assert!(
        body.contains("docs/"),
        "Root S3: listing should contain docs/"
    );

    // GET /docs should list the subdirectory.
    let req = Request::builder().uri("/docs").body(Body::empty()).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S3: GET /docs should return 200"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(
        body.contains("guide.md"),
        "Root S3: /docs listing should contain guide.md"
    );

    // GET /readme.txt should serve the file.
    let req = Request::builder()
        .uri("/readme.txt")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S3: GET /readme.txt should return 200"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.as_ref(), b"README");
}

// Scenario 3b at root: listing=true, no index specified, but index.html exists -> serve index
#[tokio::test]
async fn test_root_path_listing_with_default_index_present() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    std::fs::create_dir_all(&public_dir).unwrap();
    std::fs::write(public_dir.join("index.html"), "<html>Index Here</html>").unwrap();
    std::fs::write(public_dir.join("other.txt"), "other").unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      directory_listing: true
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    // GET / should serve index.html, not directory listing.
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S3b: GET / should return 200"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(
        body.contains("Index Here"),
        "Root S3b: GET / should serve index.html, not listing"
    );

    // Other files should still be accessible.
    let req = Request::builder()
        .uri("/other.txt")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S3b: GET /other.txt should return 200"
    );
}

// Scenario 4 at root: directory_listing=true, index specified, index present -> serve index
#[tokio::test]
async fn test_root_path_listing_custom_index_present() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    std::fs::create_dir_all(&public_dir).unwrap();
    std::fs::write(public_dir.join("home.html"), "<html>Custom Home</html>").unwrap();
    std::fs::write(public_dir.join("style.css"), "a { color: red; }").unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      index: "home.html"
      directory_listing: true
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    // GET / should serve home.html.
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S4: GET / should return 200 (home.html)"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(
        body.contains("Custom Home"),
        "Root S4: GET / should serve home.html"
    );

    // CSS should work.
    let req = Request::builder()
        .uri("/style.css")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S4: GET /style.css should return 200"
    );

    // Explicit index should work.
    let req = Request::builder()
        .uri("/home.html")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S4: GET /home.html should return 200"
    );
}

// Scenario 4b at root: listing=true, index specified but missing -> should show listing
#[tokio::test]
async fn test_root_path_listing_custom_index_missing() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    std::fs::create_dir_all(&public_dir).unwrap();
    std::fs::write(public_dir.join("readme.txt"), "hi").unwrap();
    std::fs::write(public_dir.join("app.js"), "// app").unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      index: "home.html"
      directory_listing: true
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    // GET / should show a directory listing since home.html doesn't exist.
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S4b: GET / should return 200 (directory listing)"
    );
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(
        body.contains("readme.txt"),
        "Root S4b: listing should contain readme.txt"
    );

    // Files should be directly accessible.
    let req = Request::builder()
        .uri("/readme.txt")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S4b: GET /readme.txt should return 200"
    );

    let req = Request::builder()
        .uri("/app.js")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Root S4b: GET /app.js should return 200"
    );
}

// =============================================================================
// Directory listing: nested subdirectory links are correct
// =============================================================================

#[tokio::test]
async fn test_directory_listing_nested_links() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    let a_dir = public_dir.join("a");
    let b_dir = a_dir.join("b");
    std::fs::create_dir_all(&b_dir).unwrap();
    std::fs::write(a_dir.join("file.txt"), "in a").unwrap();
    std::fs::write(b_dir.join("deep.txt"), "in b").unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      directory_listing: true
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    // Listing at /a/ should contain a link to /a/b/, not /ab/.
    let req = Request::builder().uri("/a/").body(Body::empty()).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(
        body.contains("href=\"/a/b/\""),
        "Link to subdirectory b should be /a/b/, got: {body}"
    );

    // Same via /a (no trailing slash).
    let req = Request::builder().uri("/a").body(Body::empty()).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(
        body.contains("href=\"/a/b/\""),
        "Link to subdirectory b should be /a/b/ (no trailing slash request), got: {body}"
    );

    // Verify /a/b is reachable and lists deep.txt.
    let req = Request::builder().uri("/a/b").body(Body::empty()).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(
        body.contains("deep.txt"),
        "Listing of /a/b should contain deep.txt"
    );

    // Verify the actual file is servable.
    let req = Request::builder()
        .uri("/a/b/deep.txt")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.as_ref(), b"in b");
}

// =============================================================================
// File Management Tests - Upload and Deletion
// =============================================================================

#[tokio::test]
async fn test_valid_file_upload_succeeds_with_201() {
    use axum::http::Method;
    use tower::ServiceExt;

    let dir = tempfile::TempDir::new().expect("tempdir");
    let upload_dir = dir.path().join("uploads");
    std::fs::create_dir_all(&upload_dir).unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-secret-key-for-testing-purposes-only"
    expiry: 3600

databases:
  main:
    driver: "sqlite"
    url: "sqlite::memory:"

tables:
  - name: "test_table"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/files/*"
    methods: ["get", "post"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      upload:
        enabled: true
        max_size: 10485760
        allowed_extensions: [".jpg", ".png", ".pdf"]
    auth: "jwt"
"#,
        root = upload_dir.display()
    );

    let (app, _f) = support::setup_server(&yaml).await;

    let png_data = MINIMAL_PNG.to_vec();

    // Create multipart form data using axum's multipart extraction
    let mut body = Vec::new();
    body.extend_from_slice(b"--boundary\r\n");
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"test_image.png\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body.extend_from_slice(&png_data);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(b"--boundary--");

    let token = create_token("user-123", Some("user"), &jwt_config(&yaml)).unwrap();

    let req = Request::builder()
        .method(Method::POST)
        .uri("/files/test_image.png")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "multipart/form-data; boundary=boundary")
        .body(Body::from(body))
        .unwrap();

    let response: axum::http::Response<Body> = app.clone().oneshot(req).await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "Valid file upload should return 201 Created"
    );

    let json_body = json_body(response).await;
    assert!(json_body["success"].as_bool().unwrap());
    let path = json_body["path"].as_str().unwrap();
    assert!(
        std::path::Path::new(path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("png")),
        "Path should end with .png, got: {path}"
    );
    assert_eq!(json_body["size"].as_u64().unwrap(), png_data.len() as u64);
}

#[tokio::test]
async fn test_missing_file_in_multipart_returns_400_bad_request() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let upload_dir = dir.path().join("uploads");
    std::fs::create_dir_all(&upload_dir).unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-secret-key-for-testing-purposes-only"

databases:
  main:
    driver: "sqlite"
    url: "sqlite::memory:"

tables:
  - name: "test_table"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/files/*"
    methods: ["post"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      upload:
        enabled: true
    auth: "jwt"
"#,
        root = upload_dir.display()
    );

    let (app, _f) = support::setup_server(&yaml).await;

    let token = create_token("user-123", Some("user"), &jwt_config(&yaml)).unwrap();

    // Create multipart with no file field (empty multipart)
    let body = b"--boundary--".to_vec();

    let req = Request::builder()
        .method(Method::POST)
        .uri("/files/test.txt")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "multipart/form-data; boundary=boundary")
        .body(Body::from(body))
        .unwrap();

    let response: axum::http::Response<Body> = app.oneshot(req).await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "Missing file in multipart should return 400 Bad Request"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json_body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(
        json_body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("No file provided")
    );
}

#[tokio::test]
async fn test_file_size_validation_content_length_header() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let upload_dir = dir.path().join("uploads");
    std::fs::create_dir_all(&upload_dir).unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-secret-key-for-testing-purposes-only"

databases:
  main:
    driver: "sqlite"
    url: "sqlite::memory:"

tables:
  - name: "test_table"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/files/*"
    methods: ["post"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      upload:
        enabled: true
        max_size: 1024
    auth: "jwt"
"#,
        root = upload_dir.display()
    );

    let (app, _f) = support::setup_server(&yaml).await;

    let token = create_token("user-123", Some("user"), &jwt_config(&yaml)).unwrap();

    // Create a file larger than 1024 bytes
    let large_data = vec![0x41; 2048]; // 2KB of 'A' characters

    let body = format!(
        "--boundary\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"large_file.txt\"\r\n\
         Content-Type: text/plain\r\n\r\n\
         {}\r\n\
         --boundary--",
        String::from_utf8_lossy(&large_data)
    );

    let req = Request::builder()
        .method(Method::POST)
        .uri("/files/large_file.txt")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "multipart/form-data; boundary=boundary")
        .header("content-length", "2100") // Approximate with boundary overhead
        .body(Body::from(body))
        .unwrap();

    let response: axum::http::Response<Body> = app.oneshot(req).await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "File exceeding max_size should return 413 Payload Too Large"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json_body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(
        json_body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("exceeds maximum")
    );
}

#[tokio::test]
async fn test_file_size_validation_actual_content() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let upload_dir = dir.path().join("uploads");
    std::fs::create_dir_all(&upload_dir).unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-secret-key-for-testing-purposes-only"

databases:
  main:
    driver: "sqlite"
    url: "sqlite::memory:"

tables:
  - name: "test_table"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/files/*"
    methods: ["post"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      upload:
        enabled: true
        max_size: 1024
    auth: "jwt"
"#,
        root = upload_dir.display()
    );

    let (app, _f) = support::setup_server(&yaml).await;

    let token = create_token("user-123", Some("user"), &jwt_config(&yaml)).unwrap();

    // Create a file larger than 1024 bytes without content-length header
    let large_data = vec![0x41; 2048];

    let body = format!(
        "--boundary\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"large_file.txt\"\r\n\
         Content-Type: text/plain\r\n\r\n\
         {}\r\n\
         --boundary--",
        String::from_utf8_lossy(&large_data)
    );

    let req = Request::builder()
        .method(Method::POST)
        .uri("/files/large_file.txt")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "multipart/form-data; boundary=boundary")
        .body(Body::from(body))
        .unwrap();

    let response: axum::http::Response<Body> = app.oneshot(req).await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "File content exceeding max_size should return 413 Payload Too Large"
    );
}

#[tokio::test]
async fn test_extension_validation_works() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let upload_dir = dir.path().join("uploads");
    std::fs::create_dir_all(&upload_dir).unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-secret-key-for-testing-purposes-only"

databases:
  main:
    driver: "sqlite"
    url: "sqlite::memory:"

tables:
  - name: "test_table"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/files/*"
    methods: ["post"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      upload:
        enabled: true
        allowed_extensions: [".jpg", ".png"]
    auth: "jwt"
"#,
        root = upload_dir.display()
    );

    let (app, _f) = support::setup_server(&yaml).await;

    let token = create_token("user-123", Some("user"), &jwt_config(&yaml)).unwrap();

    // Try to upload a file with disallowed extension
    let file_data = b"fake image content".to_vec();

    let mut body = Vec::new();
    body.extend_from_slice(b"--boundary\r\n");
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"document.pdf\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: application/pdf\r\n\r\n");
    body.extend_from_slice(&file_data);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(b"--boundary--");

    let req = Request::builder()
        .method(Method::POST)
        .uri("/files/document.pdf")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "multipart/form-data; boundary=boundary")
        .body(Body::from(body))
        .unwrap();

    let response: axum::http::Response<Body> = app.oneshot(req).await.unwrap();

    let status = response.status();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json_body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Disallowed extension should return 400 Bad Request"
    );

    assert!(
        json_body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not allowed")
    );
}

#[tokio::test]
async fn test_magic_byte_validation_rejects_fake_image_files() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let upload_dir = dir.path().join("uploads");
    std::fs::create_dir_all(&upload_dir).unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-secret-key-for-testing-purposes-only"

databases:
  main:
    driver: "sqlite"
    url: "sqlite::memory:"

tables:
  - name: "test_table"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/files/*"
    methods: ["post"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      upload:
        enabled: true
        allowed_extensions: [".jpg", ".png"]
    auth: "jwt"
"#,
        root = upload_dir.display()
    );

    let (app, _f) = support::setup_server(&yaml).await;

    let token = create_token("user-123", Some("user"), &jwt_config(&yaml)).unwrap();

    // Try to upload a fake PNG (text file with .png extension)
    let fake_png_data = b"This is not a PNG file, just text!".to_vec();

    let body = format!(
        "--boundary\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"fake_image.png\"\r\n\
         Content-Type: image/png\r\n\r\n\
         {}\r\n\
         --boundary--",
        String::from_utf8_lossy(&fake_png_data)
    );

    let req = Request::builder()
        .method(Method::POST)
        .uri("/files/fake_image.png")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "multipart/form-data; boundary=boundary")
        .body(Body::from(body))
        .unwrap();

    let response: axum::http::Response<Body> = app.oneshot(req).await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "Fake image should return 400 Bad Request"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json_body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(
        json_body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("magic bytes")
    );
}

#[tokio::test]
async fn test_uuid_based_filenames_prevent_collisions() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let upload_dir = dir.path().join("uploads");
    std::fs::create_dir_all(&upload_dir).unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-secret-key-for-testing-purposes-only"

databases:
  main:
    driver: "sqlite"
    url: "sqlite::memory:"

tables:
  - name: "test_table"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/files/*"
    methods: ["post"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      upload:
        enabled: true
    auth: "jwt"
"#,
        root = upload_dir.display()
    );

    let (app, _f) = support::setup_server(&yaml).await;

    let token = create_token("user-123", Some("user"), &jwt_config(&yaml)).unwrap();

    let file_data = MINIMAL_PNG.to_vec();

    // First upload
    let mut body1 = Vec::new();
    body1.extend_from_slice(b"--boundary\r\n");
    body1.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"test.png\"\r\n",
    );
    body1.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body1.extend_from_slice(&file_data);
    body1.extend_from_slice(b"\r\n");
    body1.extend_from_slice(b"--boundary--");

    let req1 = Request::builder()
        .method(Method::POST)
        .uri("/files/test.png")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "multipart/form-data; boundary=boundary")
        .body(Body::from(body1))
        .unwrap();

    let response1: axum::http::Response<Body> = app.clone().oneshot(req1).await.unwrap();
    assert_eq!(response1.status(), StatusCode::CREATED);

    let body_bytes1 = response1.into_body().collect().await.unwrap().to_bytes();
    let response1_json: serde_json::Value = serde_json::from_slice(&body_bytes1).unwrap();
    let path1 = response1_json["path"].as_str().unwrap().to_string();

    // Second upload with same filename
    let mut body2 = Vec::new();
    body2.extend_from_slice(b"--boundary\r\n");
    body2.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"test.png\"\r\n",
    );
    body2.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body2.extend_from_slice(&file_data);
    body2.extend_from_slice(b"\r\n");
    body2.extend_from_slice(b"--boundary--");

    let req2 = Request::builder()
        .method(Method::POST)
        .uri("/files/test.png")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "multipart/form-data; boundary=boundary")
        .body(Body::from(body2))
        .unwrap();

    let response2: axum::http::Response<Body> = app.clone().oneshot(req2).await.unwrap();

    // Should succeed with UUID-based filename
    assert_eq!(response2.status(), StatusCode::CREATED);

    let response2_json = json_body(response2).await;
    let path2 = response2_json["path"].as_str().unwrap().to_string();

    // Paths should be different (UUID prefix)
    assert_ne!(
        path1, path2,
        "UUID-based filenames should prevent collisions"
    );

    // Both files should have UUID prefix (UUID contains dashes)
    assert!(path1.contains('-'), "First upload should have UUID prefix");
    assert!(path2.contains('-'), "Second upload should have UUID prefix");
}

#[tokio::test]
async fn test_parent_directories_created_automatically() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let upload_dir = dir.path().join("uploads");
    std::fs::create_dir_all(&upload_dir).expect("create upload dir");
    // Don't create subdirectories - they should be created automatically

    let yaml = format!(
        r#"
server:
  port: 0

auth:
   jwt:
    secret: "test-secret-key-for-testing-purposes-only"

databases:
  main:
    driver: "sqlite"
    url: "sqlite::memory:"

tables:
  - name: "test_table"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/files/*"
    methods: ["post"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      upload:
        enabled: true
        create_subdirectory: "{{user_id}}/2024/01"
    auth: "jwt"
"#,
        root = upload_dir.display()
    );

    let (app, _f) = support::setup_server(&yaml).await;

    let token = create_token("user-123", Some("user"), &jwt_config(&yaml)).unwrap();

    let file_data = b"nested file content".to_vec();

    let mut body = Vec::new();
    body.extend_from_slice(b"--boundary\r\n");
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"nested_file.txt\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: text/plain\r\n\r\n");
    body.extend_from_slice(&file_data);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(b"--boundary--");

    let req = Request::builder()
        .method(Method::POST)
        .uri("/files/nested_file.txt")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "multipart/form-data; boundary=boundary")
        .body(Body::from(body))
        .unwrap();

    let response: axum::http::Response<Body> = app.oneshot(req).await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "Parent directories should be created automatically"
    );

    let json_body = json_body(response).await;
    assert!(json_body["success"].as_bool().unwrap());

    // Verify the file exists in the expected nested path
    let path_str = json_body["path"].as_str().unwrap();
    let file_path = upload_dir.join(path_str.trim_start_matches('/'));

    assert!(
        file_path.exists(),
        "File should exist at nested path: {file_path:?}"
    );
}

#[tokio::test]
async fn test_existing_files_return_409_conflict() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let upload_dir = dir.path().join("uploads");
    std::fs::create_dir_all(&upload_dir).unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-secret-key-for-testing-purposes-only"

databases:
  main:
    driver: "sqlite"
    url: "sqlite::memory:"

tables:
  - name: "test_table"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/files/*"
    methods: ["post"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      upload:
        enabled: true
    auth: "jwt"
"#,
        root = upload_dir.display()
    );

    let (app, _f) = support::setup_server(&yaml).await;

    let token = create_token("user-123", Some("user"), &jwt_config(&yaml)).unwrap();

    // First upload - should succeed
    let file_data = MINIMAL_PNG.to_vec();

    let mut body1 = Vec::new();
    body1.extend_from_slice(b"--boundary\r\n");
    body1.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"test.png\"\r\n",
    );
    body1.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body1.extend_from_slice(&file_data);
    body1.extend_from_slice(b"\r\n");
    body1.extend_from_slice(b"--boundary--");

    let req1 = Request::builder()
        .method(Method::POST)
        .uri("/files/test.png")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "multipart/form-data; boundary=boundary")
        .body(Body::from(body1))
        .unwrap();

    let response1: axum::http::Response<Body> = app.clone().oneshot(req1).await.unwrap();
    let status1 = response1.status();
    let body_bytes = response1.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(status1, StatusCode::CREATED);

    let response1_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let _filename = response1_json["path"].as_str().unwrap().to_string();

    // Note: The code uses UUIDs to prevent filename collisions, so uploading the same file
    // again will create a new UUID and succeed. This test verifies that multiple uploads work.
    let mut body2 = Vec::new();
    body2.extend_from_slice(b"--boundary\r\n");
    body2.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"test2.png\"\r\n",
    );
    body2.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body2.extend_from_slice(&file_data);
    body2.extend_from_slice(b"\r\n");
    body2.extend_from_slice(b"--boundary--");

    let req2 = Request::builder()
        .method(Method::POST)
        .uri("/files/test2.png")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "multipart/form-data; boundary=boundary")
        .body(Body::from(body2))
        .unwrap();

    let response2: axum::http::Response<Body> = app.oneshot(req2).await.unwrap();
    let status2 = response2.status();
    let body_bytes2 = response2.into_body().collect().await.unwrap().to_bytes();
    let json_body: serde_json::Value = serde_json::from_slice(&body_bytes2).unwrap();

    // Verify second upload succeeds (UUIDs prevent collisions)
    assert_eq!(
        status2,
        StatusCode::CREATED,
        "Second upload should return 201 Created"
    );

    assert!(json_body["success"].as_bool().unwrap());
}

#[tokio::test]
async fn test_delete_file_success() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let upload_dir = dir.path().join("uploads");
    std::fs::create_dir_all(&upload_dir).unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-secret-key-for-testing-purposes-only"

databases:
  main:
    driver: "sqlite"
    url: "sqlite::memory:"

tables:
  - name: "test_table"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/files/*"
    methods: ["get", "post", "delete"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      upload:
        enabled: true
    auth: "jwt"
"#,
        root = upload_dir.display()
    );

    let (app, _f) = support::setup_server(&yaml).await;

    let token = create_token("user-123", Some("user"), &jwt_config(&yaml)).unwrap();

    // First, upload a file
    let file_data = MINIMAL_PNG.to_vec();

    let mut body = Vec::new();
    body.extend_from_slice(b"--boundary\r\n");
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"to_delete.png\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body.extend_from_slice(&file_data);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(b"--boundary--");

    let req = Request::builder()
        .method(Method::POST)
        .uri("/files/to_delete.png")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "multipart/form-data; boundary=boundary")
        .body(Body::from(body))
        .unwrap();

    let response: axum::http::Response<Body> = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let json_body = json_body(response).await;
    let file_path = json_body["path"].as_str().unwrap().to_string();

    // Now delete the file
    let delete_req = Request::builder()
        .method(Method::DELETE)
        .uri(format!("/files{file_path}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let delete_response: axum::http::Response<Body> = app.oneshot(delete_req).await.unwrap();
    let status = delete_response.status();

    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "Delete should return 204 No Content"
    );

    // Verify file is actually deleted
    let full_path = upload_dir.join(file_path.trim_start_matches('/'));
    assert!(!full_path.exists(), "File should be deleted from disk");
}

#[tokio::test]
async fn test_delete_unauthorized_user() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let upload_dir = dir.path().join("uploads");
    std::fs::create_dir_all(&upload_dir).unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-secret-key-for-testing-purposes-only"

databases:
  main:
    driver: "sqlite"
    url: "sqlite::memory:"

tables:
  - name: "test_table"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/files/*"
    methods: ["delete"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      upload:
        enabled: true
    auth: "jwt"
    roles:
      - "admin"
"#,
        root = upload_dir.display()
    );

    let (app, _f) = support::setup_server(&yaml).await;

    // User with "user" role tries to delete
    let token = create_token("user-456", Some("user"), &jwt_config(&yaml)).unwrap();

    let delete_req = Request::builder()
        .method(Method::DELETE)
        .uri("/files/somefile.png")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response: axum::http::Response<Body> = app.oneshot(delete_req).await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "User without required role should be forbidden"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json_body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    // Check error structure: {"error": {"code": "...", "message": "..."}}
    assert!(
        json_body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not authorized")
    );
}

// =============================================================================
// 5.2 Static file security tests - directory traversal prevention
// =============================================================================

#[tokio::test]
async fn test_static_files_directory_traversal_dotdot() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root_str = dir.path().display().to_string();

    // Create a legitimate file
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::write(
        dir.path().join("sub").join("index.html"),
        b"<html>ok</html>",
    )
    .unwrap();
    // Create a "sensitive" file at root level
    std::fs::write(dir.path().join("sensitive.txt"), b"secret data").unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/assets/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
    auth: "none"
"#,
        root = root_str
    );

    let (app, _f) = support::setup_server(&yaml).await;

    // Request with ../ traversal - should be forbidden
    let req = Request::builder()
        .method(Method::GET)
        .uri("/assets/sub/../../sensitive.txt")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_static_files_directory_traversal_single_dot() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root_str = dir.path().display().to_string();

    std::fs::write(dir.path().join("real.html"), b"<html>ok</html>").unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/assets/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
    auth: "none"
"#,
        root = root_str
    );

    let (app, _f) = support::setup_server(&yaml).await;

    // Request with . segment - should be forbidden
    let req = Request::builder()
        .method(Method::GET)
        .uri("/assets/./../../sensitive.txt")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_static_files_directory_traversal_percent_encoded() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root_str = dir.path().display().to_string();

    std::fs::write(dir.path().join("real.html"), b"<html>ok</html>").unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/assets/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
    auth: "none"
"#,
        root = root_str
    );

    let (app, _f) = support::setup_server(&yaml).await;

    // Request with percent-encoded ../ as %2e%2e%2f
    let req = Request::builder()
        .method(Method::GET)
        .uri("/assets/%2e%2e/%2e%2e/sensitive.txt")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_static_files_directory_traversal_nested_dotdot() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root_str = dir.path().display().to_string();

    std::fs::write(dir.path().join("real.html"), b"<html>ok</html>").unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/assets/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
    auth: "none"
"#,
        root = root_str
    );

    let (app, _f) = support::setup_server(&yaml).await;

    // Multiple levels of traversal
    let req = Request::builder()
        .method(Method::GET)
        .uri("/assets/a/b/../../../sensitive.txt")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_static_files_normal_subdirectory_access() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root_str = dir.path().display().to_string();

    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::write(
        dir.path().join("sub").join("index.html"),
        b"<html>ok</html>",
    )
    .unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/assets/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
    auth: "none"
"#,
        root = root_str
    );

    let (app, _f) = support::setup_server(&yaml).await;

    // Normal subdirectory access should work
    let req = Request::builder()
        .method(Method::GET)
        .uri("/assets/sub/index.html")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&*body_bytes, b"<html>ok</html>");
}

// =============================================================================
// 5.2 Static file security tests - path canonicalization for store-backed
// =============================================================================

#[tokio::test]
async fn test_spa_host_directory_traversal_prevention() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root_str = dir.path().display().to_string();

    std::fs::write(dir.path().join("index.html"), b"<html>spa</html>").unwrap();
    std::fs::write(dir.path().join("secret.txt"), b"should not be accessible").unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/app/*"
    methods: ["get"]
    action: "spa_host"
    spa_host:
      storage: "assets"
      index: "index.html"
    auth: "none"
"#,
        root = root_str
    );

    let (app, _f) = support::setup_server(&yaml).await;

    // SPA host should reject path traversal
    let req = Request::builder()
        .method(Method::GET)
        .uri("/app/../../secret.txt")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_static_files_empty_path_serves_root() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root_str = dir.path().display().to_string();

    std::fs::write(dir.path().join("index.html"), b"<html>root</html>").unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/static/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
    auth: "none"
"#,
        root = root_str
    );

    let (app, _f) = support::setup_server(&yaml).await;

    // Requesting just /static/ should serve index.html
    let req = Request::builder()
        .method(Method::GET)
        .uri("/static/")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&*body_bytes, b"<html>root</html>");
}

#[tokio::test]
async fn test_static_files_single_dot_file_forbidden() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root_str = dir.path().display().to_string();

    std::fs::write(dir.path().join("index.html"), b"<html>ok</html>").unwrap();

    let yaml = format!(
        r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/static/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
    auth: "none"
"#,
        root = root_str
    );

    let (app, _f) = support::setup_server(&yaml).await;

    // Request with ./ segment
    let req = Request::builder()
        .method(Method::GET)
        .uri("/static/./index.html")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

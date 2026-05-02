/// Integration tests for YAML-defined API endpoints.
///
/// CRUD edge-case tests run across all enabled backends (sqlite, postgres,
/// mysql). Basic CRUD operations and pagination/filtering/sorting are covered
/// in `db_backends.rs` and `db_api_features.rs`. Static file tests are
/// backend-independent.
mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use support::db::{TestDatabase, enabled_backends};
use support::{CRUD_CONFIG, json_body};
use tower::ServiceExt;

// =============================================================================
// CRUD: List (GET /api/posts)
// =============================================================================

#[tokio::test]
async fn test_crud_list_empty() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_list_empty");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        let req = Request::builder()
            .uri("/api/posts")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");

        let json = json_body(response).await;
        let data = json["data"].as_array().unwrap();
        assert!(data.is_empty(), "backend: {backend}");
    }
}

// =============================================================================
// CRUD: Get single (GET /api/posts/{id})
// =============================================================================

#[tokio::test]
async fn test_crud_get_one() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_get_one");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        // Create a post.
        let req = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "title": "Test Post",
                    "body": "content",
                    "author": "Charlie"
                })
                .to_string(),
            ))
            .unwrap();
        let _ = app.clone().oneshot(req).await.unwrap();

        // Get post by ID.
        let req = Request::builder()
            .uri("/api/posts/1")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");

        let json = json_body(response).await;
        assert_eq!(json["title"], "Test Post", "backend: {backend}");
        assert_eq!(json["author"], "Charlie", "backend: {backend}");
    }
}

#[tokio::test]
async fn test_crud_get_one_not_found() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_get_not_found");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        let req = Request::builder()
            .uri("/api/posts/999")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "backend: {backend}"
        );
    }
}

// =============================================================================
// CRUD: Update not found (PUT /api/posts/{id})
// =============================================================================

#[tokio::test]
async fn test_crud_update_not_found() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_update_not_found");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        let req = Request::builder()
            .method("PUT")
            .uri("/api/posts/999")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({"title": "Nope"}).to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "backend: {backend}"
        );
    }
}

// =============================================================================
// CRUD: Delete not found (DELETE /api/posts/{id})
// =============================================================================

#[tokio::test]
async fn test_crud_delete_not_found() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_delete_not_found");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        let req = Request::builder()
            .method("DELETE")
            .uri("/api/posts/999")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "backend: {backend}"
        );
    }
}

// =============================================================================
// CRUD: Invalid requests
// =============================================================================

#[tokio::test]
async fn test_crud_create_no_body() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_no_body");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        let req = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "backend: {backend}"
        );
    }
}

#[tokio::test]
async fn test_crud_create_ignores_non_writable_field() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_non_writable");
        let (app, _state, _pool) = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        // Try to set "id" which is not in writable_fields.
        let req = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"id": 999, "title": "T", "body": "", "author": "X"}).to_string(),
            ))
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED, "backend: {backend}");

        // Verify the id was NOT set to 999.
        let req = Request::builder()
            .uri("/api/posts/1")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "backend: {backend}");
    }
}

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

    let req = Request::builder()
        .uri("/static/nonexistent.txt")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_static_spa_fallback() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let public_dir = dir.path().join("public");
    std::fs::create_dir_all(&public_dir).unwrap();
    std::fs::write(public_dir.join("index.html"), "<html>SPA</html>").unwrap();

    let yaml_tmpl = r#"
server:
  port: 0

endpoints:
  - path: "/app/*"
    methods: ["get"]
    action: "static"
    static_files:
      root: "{root}"
      index: "index.html"
      spa_fallback: true
    auth: "none"
"#;
    let yaml = yaml_tmpl.replace("{root}", &public_dir.display().to_string());
    let (app, _f) = support::setup_server(&yaml).await;

    // Request a non-existent path - SPA fallback should return index.html.
    let req = Request::builder()
        .uri("/app/deep/route")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("SPA"));
}

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

endpoints:
  - path: "/static/*"
    methods: ["get"]
    action: "static"
    static_files:
      root: "{root}"
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

endpoints:
  - path: "/site"
    methods: ["get"]
    action: "static"
    static_files:
      root: "{root}"
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

endpoints:
  - path: "/site/*"
    methods: ["get"]
    action: "static"
    static_files:
      root: "{root}"
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

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static"
    static_files:
      root: "{root}"
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

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static"
    static_files:
      root: "{root}"
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

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static"
    static_files:
      root: "{root}"
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

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static"
    static_files:
      root: "{root}"
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

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static"
    static_files:
      root: "{root}"
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

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static"
    static_files:
      root: "{root}"
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

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static"
    static_files:
      root: "{root}"
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

endpoints:
  - path: "/"
    methods: ["get"]
    action: "static"
    static_files:
      root: "{root}"
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

use main_serve::config::types::JwtConfig;
use std::io::Write;
use std::path::PathBuf;
use tempfile::NamedTempFile;

/// Helper: Write YAML to a temp file and load it.
pub fn load_yaml(yaml: &str) -> Result<main_serve::config::AppConfig, main_serve::error::AppError> {
    let mut f = NamedTempFile::new().expect("failed to create temp file");
    f.write_all(yaml.as_bytes()).expect("failed to write");
    main_serve::config::load_config(f.path())
}

/// Helper: Extract JWT config from YAML.
///
/// Parses the auth.jwt section from a YAML config string and returns a `JwtConfig`
/// that matches the server's configuration. This ensures tokens created with
/// this config will validate correctly against endpoints using that YAML config.
pub fn jwt_config(yaml: &str) -> JwtConfig {
    let config = load_yaml(yaml).expect("failed to parse YAML for jwt_config");
    config
        .auth
        .jwt
        .expect("YAML config must have auth.jwt configured")
}

/// Populate a directory with test site files. Returns the root path.
///
/// Creates the `public/` subdirectory under `dir` and writes all provided
/// (relative_path, content) pairs into it.
pub fn write_site_files(dir: &std::path::Path, files: &[(&str, &str)]) -> PathBuf {
    let root = dir.join("public");
    std::fs::create_dir_all(&root).unwrap();
    for (path, content) in files {
        let file_path = root.join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&file_path, content).unwrap();
    }
    root
}

/// Generate the argon2 hash for the admin password used in basic auth tests.
///
/// Uses a fixed salt so the hash is deterministic across test runs.
/// Password is `s3cureP@ss`.
pub fn basic_auth_hash() -> String {
    basic_auth_hash_with_password("s3cureP@ss")
}

/// Generate an argon2 password hash with a custom password and a fixed salt.
///
/// Uses the salt `dGVzdHNhbHR2YWx1ZQ` (decoded: `testsaltvalue`) so the hash
/// is deterministic across test runs.
pub fn basic_auth_hash_with_password(password: &str) -> String {
    use argon2::{Argon2, PasswordHasher, password_hash::SaltString};

    let salt = SaltString::from_b64("dGVzdHNhbHR2YWx1ZQ").unwrap();
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .unwrap()
        .to_string()
}

/// Minimal valid 1x1 PNG image bytes.
///
/// Returns a `&[u8]` slice for zero-copy usage. Suitable for upload tests
/// that need a valid PNG fixture.
pub const MINIMAL_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
    0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1 pixel
    0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41,
    0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D,
    0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// Seed a JSONB-enabled posts table with records containing metadata.
///
/// Posts each have a `title`, `author`, and a `metadata` field parsed from
/// JSON strings. All seeds are posted to `/api/posts`.
pub async fn seed_jsonb_posts(app: &axum::Router, posts: &[(&str, &str, &str)]) {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    for (title, author, metadata_json) in posts {
        let metadata_value: serde_json::Value =
            serde_json::from_str(metadata_json).unwrap_or_else(|e| {
                panic!(
                    "Failed to parse metadata JSON for post '{title}': {e}\nJSON input: {metadata_json}"
                );
            });

        let req = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "title": title,
                    "author": author,
                    "metadata": metadata_value
                })
                .to_string(),
            ))
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "Failed to seed post: '{title}'"
        );
    }
}

/// Seed a simple posts table with (title, author) pairs.
///
/// Posts are created via POST `/api/posts` with `body: ""`.
/// Returns the list of IDs created (in order).
pub async fn seed_posts(app: &axum::Router, posts: &[(&str, &str)]) -> Vec<i64> {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let mut ids = Vec::new();
    for (title, author) in posts {
        let req = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "title": title,
                    "body": "",
                    "author": author,
                })
                .to_string(),
            ))
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if let Some(id) = json
            .get("id")
            .or_else(|| json.get("data").and_then(|d| d.get("id")))
        {
            ids.push(id.as_i64().unwrap());
        }
    }
    ids
}

/// Seed a JSONB-enabled posts table with 5 default posts for filter/sort testing.
///
/// Posts include rich metadata with roles (alpha/beta/gamma/delta/epsilon),
/// statuses, nested user objects with ages, and tag arrays. Seeded via
/// POST `/api/posts`.
pub async fn seed_default_jsonb_posts(app: &axum::Router) {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    let seed_data = vec![
        serde_json::json!({
            "title": "Post Alpha",
            "author": "alice",
            "metadata": serde_json::json!({
                "role": "alpha",
                "status": "active",
                "user": {"age": 30}
            }),
            "tags": ["rust", "web"]
        }),
        serde_json::json!({
            "title": "Post Beta",
            "author": "bob",
            "metadata": serde_json::json!({
                "role": "beta",
                "status": "active",
                "user": {"age": 25}
            }),
            "tags": ["python", "ml"]
        }),
        serde_json::json!({
            "title": "Post Gamma",
            "author": "charlie",
            "metadata": serde_json::json!({
                "role": "gamma",
                "status": "inactive",
                "user": {"age": 35}
            }),
            "tags": ["go", "systems"]
        }),
        serde_json::json!({
            "title": "Post Delta",
            "author": "alice",
            "metadata": serde_json::json!({
                "role": "delta",
                "status": "active",
                "user": {"age": 28}
            }),
            "tags": ["rust", "ml"]
        }),
        serde_json::json!({
            "title": "Post Epsilon",
            "author": "bob",
            "metadata": serde_json::json!({
                "role": "epsilon",
                "status": "draft",
                "user": {"age": 22}
            }),
            "tags": ["javascript"]
        }),
    ];
    for data in &seed_data {
        let req = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(data.to_string()))
            .unwrap();
        let _response = app.clone().oneshot(req).await.unwrap();
    }
}

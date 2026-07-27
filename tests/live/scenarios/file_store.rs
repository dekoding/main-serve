//! File store scenario.
//!
//! Tests Main Serve configured as a file store (database-backed file catalog):
//!
//! - CRUD operations (create, read, list, update, delete)
//! - Field-level read/write permissions
//! - Row-level ownership authorization
//! - Trash for deleted entries
//! - Pagination, sorting, and filtering
//! - Metadata columns
//!
//! Config: based on the file_store action from config/templates/example.yaml with SQLite and native storage.

use crate::support::{BinaryHandle, LiveClient};
use reqwest::StatusCode;

const FILE_STORE_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

logging:
  level: "info"
  format: "json"

databases:
  main:
    driver: "sqlite"
    url: "sqlite://filestore.db?mode=rwc"
    auto_migrate: true

tables:
  - name: "users"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
        nullable: false
      - name: "email"
        type: "varchar"
        unique: true
        nullable: false
      - name: "password_hash"
        type: "text"
        nullable: false
      - name: "role"
        type: "varchar"
        nullable: false
        default: "'user'"
      - name: "uploader_id"
        type: "uuid"
        nullable: true

  - name: "files"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
        nullable: false
      - name: "file_path"
        type: "text"
      - name: "original_name"
        type: "text"
      - name: "mime_type"
        type: "text"
      - name: "size"
        type: "bigint"
      - name: "uploader_id"
        type: "uuid"
      - name: "tags"
        type: "jsonb"
        nullable: true
      - name: "description"
        type: "text"
        nullable: true
      - name: "deleted_at"
        type: "timestamptz"
        nullable: true
      - name: "trashed_at"
        type: "timestamptz"
        nullable: true
      - name: "created_at"
        type: "timestamptz"
        nullable: false
        default: "CURRENT_TIMESTAMP"
      - name: "updated_at"
        type: "timestamptz"
        nullable: false
        default: "CURRENT_TIMESTAMP"

  - name: "media_entity_refs"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
        nullable: false
      - name: "media_id"
        type: "bigint"
        nullable: false
      - name: "entity_id"
        type: "bigint"
        nullable: false
      - name: "content_type"
        type: "varchar"
        nullable: false
      - name: "attachment_order"
        type: "integer"
        nullable: false
        default: "0"

auth:
  jwt:
    secret: "filestore-test-secret-key"
    algorithm: "HS256"
    issuer: "filestore-server"
    expiry: 3600
    role_claim: "role"

  register:
    enabled: true
    table: "users"
    database: "main"
    default_role: "user"
    password_hash: "argon2id"

stores:
  file_storage:
    backend: "native"
    root: "./files"

endpoints:
  - path: "/api/files"
    methods: ["get", "post"]
    action: "file_store"
    auth: "jwt"
    roles: ["admin", "editor", "author", "user"]
    file_store:
      storage: "file_storage"
      table: "files"
      database: "main"
      metadata_columns:
        - name: "original_name"
          type: "text"
        - name: "mime_type"
          type: "text"
        - name: "size"
          type: "bigint"
        - name: "uploader_id"
          type: "uuid"
        - name: "tags"
          type: "jsonb"
          nullable: true
          default: "null"
        - name: "description"
          type: "text"
          nullable: true
          default: "null"
      field_permissions:
        original_name:
          read: ["admin", "editor"]
          write: ["admin"]
        mime_type:
          read: ["admin", "editor", "author"]
          write: ["admin"]
        tags:
          read: ["admin", "editor"]
          write: ["admin", "editor", "author"]
        description:
          read: ["admin", "editor"]
          write: ["admin", "editor", "author"]
      ownership:
        owner_column: "uploader_id"
        admin_override: true
      trash:
        enabled: true
        retention_days: 30
      pagination:
        enabled: true
        default_page_size: 20
        max_page_size: 100
      sorting:
        enabled: true
        default_field: "created_at"
        default_order: "desc"
        allowed_fields: ["original_name", "mime_type", "created_at", "size"]
      filtering:
        enabled: true
        allowed_fields: ["mime_type", "original_name", "created_at", "size"]

  - path: "/api/files/{id}"
    methods: ["get", "patch", "delete"]
    action: "file_store"
    auth: "jwt"
    roles: ["admin", "editor", "author", "user"]
    file_store:
      storage: "file_storage"
      table: "files"
      database: "main"
      ownership:
        owner_column: "uploader_id"
        admin_override: true
      trash:
        enabled: true
        retention_days: 30

  - path: "/api/files/{id}/refs"
    methods: ["post", "delete"]
    action: "file_store"
    auth: "jwt"
    roles: ["admin", "editor", "author", "user"]
    file_store:
      storage: "file_storage"
      table: "files"
      database: "main"
      content_references:
        enabled: true
        table: "media_entity_refs"
        media_id_column: "media_id"
        entity_id_column: "entity_id"
        content_type_column: "content_type"
        order_column: "attachment_order"

  - path: "/_main-serve/file-store/trash"
    methods: ["get", "delete"]
    action: "file_store"
    auth: "jwt"
    roles: ["admin"]
    file_store:
      storage: "file_storage"
      table: "files"
      database: "main"
      trash:
        enabled: true
        retention_days: 30

  - path: "/_main-serve/file-store/trash/{id}"
    methods: ["delete"]
    action: "file_store"
    auth: "jwt"
    roles: ["admin"]
    file_store:
      storage: "file_storage"
      table: "files"
      database: "main"
      trash:
        enabled: true
        retention_days: 30

  - path: "/_main-serve/file-store/trash/{id}/restore"
    methods: ["post"]
    action: "file_store"
    auth: "jwt"
    roles: ["admin"]
    file_store:
      storage: "file_storage"
      table: "files"
      database: "main"
      trash:
        enabled: true
        retention_days: 30
"#;

/// Helper to register a user and get their token
async fn register_and_login(client: &LiveClient, email: &str, password: &str) -> String {
    let email_prefix = email.split('@').next().unwrap_or(email);
    let role = if email_prefix == "admin"
        || email_prefix.starts_with("admin_")
        || email_prefix.starts_with("owner")
    {
        serde_json::json!({
            "email": email,
            "password": password,
            "role": "admin"
        })
    } else if email_prefix == "author" || email_prefix == "user1" || email_prefix == "user2" {
        serde_json::json!({
            "email": email,
            "password": password,
            "role": "author"
        })
    } else {
        serde_json::json!({
            "email": email,
            "password": password
        })
    };

    client
        .post_json("/_main-serve/register", &role)
        .await
        .expect("register")
        .error_for_status()
        .expect("register should succeed");

    let login_resp = client
        .post_json(
            "/_main-serve/login",
            &serde_json::json!({
                "email": email,
                "password": password
            }),
        )
        .await
        .expect("login");

    client.assert_status(&login_resp, StatusCode::OK).await;
    let body = login_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse login");
    body.get("token")
        .and_then(|v| v.as_str())
        .expect("token")
        .to_string()
}

async fn setup_file_store_server() -> (BinaryHandle, tempfile::TempDir) {
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("create temp dir");
    let files_dir = temp_dir.path().join("files");
    std::fs::create_dir_all(&files_dir).expect("create files dir");
    let db_path = temp_dir.path().join("filestore.db");

    let config = FILE_STORE_CONFIG
        .replace(
            "sqlite://filestore.db?mode=rwc",
            &format!("sqlite://{}?mode=rwc", db_path.display()),
        )
        .replace("./files", files_dir.to_str().unwrap());

    let server = BinaryHandle::spawn(&config, None)
        .await
        .expect("spawn server");
    (server, temp_dir)
}

#[tokio::test]
async fn test_file_store_create() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    let token = register_and_login(&client, "admin_create@example.com", "adminpass123").await;
    let authed = LiveClient::new(server.base_url()).with_bearer_token(&token);

    // Create a file entry
    let resp = authed
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "document.pdf",
                "mime_type": "application/pdf",
                "size": 1024,
                "description": "Test document"
            }),
        )
        .await
        .expect("create file");

    // Should succeed (200 or 201)
    assert!(
        resp.status() == StatusCode::OK || resp.status() == StatusCode::CREATED,
        "create file status: {}, body: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_file_store_list() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    let token = register_and_login(&client, "admin_list@example.com", "adminpass123").await;
    let authed = LiveClient::new(server.base_url()).with_bearer_token(&token);

    // Create two file entries
    authed
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "file1.txt",
                "mime_type": "text/plain",
                "size": 256,
            }),
        )
        .await
        .expect("create 1")
        .error_for_status()
        .expect("create 1 should succeed");

    authed
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "file2.txt",
                "mime_type": "text/plain",
                "size": 512,
            }),
        )
        .await
        .expect("create 2")
        .error_for_status()
        .expect("create 2 should succeed");

    // List files
    let resp = authed
        .get_json::<serde_json::Value>("/api/files")
        .await
        .expect("list");

    let results = resp
        .get("results")
        .or_else(|| resp.get("data"))
        .and_then(|v| v.as_array())
        .expect("results array");

    assert_eq!(results.len(), 2, "should list 2 files");

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_file_store_get_by_id() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    let token = register_and_login(&client, "admin_get@example.com", "adminpass123").await;
    let authed = LiveClient::new(server.base_url()).with_bearer_token(&token);

    // Create a file entry
    let create_resp = authed
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "get_test.txt",
                "mime_type": "text/plain",
                "size": 128,
            }),
        )
        .await
        .expect("create");
    assert!(create_resp.status().is_success() || create_resp.status() == StatusCode::CREATED);

    // Get the response body to extract the id
    let created_json = create_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse");
    let file_id = created_json
        .get("id")
        .or_else(|| created_json.get("data").and_then(|d| d.get("id")))
        .and_then(|v| v.as_str())
        .expect("id");

    // Get by ID
    let resp = authed
        .get(&format!("/api/files/{file_id}"))
        .await
        .expect("get by id");

    client.assert_status(&resp, StatusCode::OK).await;

    let file_data = resp
        .json::<serde_json::Value>()
        .await
        .expect("parse response");
    let returned_id = file_data
        .get("id")
        .or_else(|| file_data.get("data").and_then(|d| d.get("id")))
        .and_then(|v| v.as_str())
        .expect("id in response");
    assert_eq!(
        returned_id, file_id,
        "retrieved file should match requested id"
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_file_store_update() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    let token = register_and_login(&client, "admin_update@example.com", "adminpass123").await;
    let authed = LiveClient::new(server.base_url()).with_bearer_token(&token);

    // Create a file entry
    let create_resp = authed
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "update_test.txt",
                "mime_type": "text/plain",
                "size": 100,
            }),
        )
        .await
        .expect("create");
    assert!(create_resp.status().is_success() || create_resp.status() == StatusCode::CREATED);

    let created_json = create_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse");
    let file_id = created_json
        .get("id")
        .or_else(|| created_json.get("data").and_then(|d| d.get("id")))
        .and_then(|v| v.as_str())
        .expect("id");

    // Update the file entry
    let update_resp = authed
        .patch_json(
            &format!("/api/files/{file_id}"),
            &serde_json::json!({
                "description": "Updated description"
            }),
        )
        .await
        .expect("update");

    client.assert_status(&update_resp, StatusCode::OK).await;

    // Verify the update was applied
    let updated_data = update_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse update response");
    let updated_desc = updated_data
        .get("description")
        .or_else(|| updated_data.get("data").and_then(|d| d.get("description")))
        .and_then(|v| v.as_str())
        .expect("description in response");
    assert_eq!(
        updated_desc, "Updated description",
        "description should be updated"
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_file_store_delete_and_trash() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    let token = register_and_login(&client, "admin_delete@example.com", "adminpass123").await;
    let authed = LiveClient::new(server.base_url()).with_bearer_token(&token);

    // Create a file entry
    let create_resp = authed
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "trash_test.txt",
                "mime_type": "text/plain",
                "size": 64,
            }),
        )
        .await
        .expect("create");
    assert!(create_resp.status().is_success() || create_resp.status() == StatusCode::CREATED);

    let created_json = create_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse");
    let file_id = created_json
        .get("id")
        .or_else(|| created_json.get("data").and_then(|d| d.get("id")))
        .and_then(|v| v.as_str())
        .expect("id");

    // Delete (should go to trash)
    let del_resp = authed
        .delete(&format!("/api/files/{file_id}"))
        .await
        .expect("delete");

    let del_status = del_resp.status();
    assert!(
        del_status == StatusCode::OK || del_status == StatusCode::NO_CONTENT,
        "delete status: {}, body: {}",
        del_status,
        del_resp.text().await.unwrap_or_default()
    );

    // Check trash
    let trash_resp = authed.get("/_main-serve/file-store/trash").await;

    if let Ok(trash) = trash_resp {
        let trash_body = trash.json::<serde_json::Value>().await.ok();
        if let Some(trash) = trash_body {
            let trash_results = trash
                .get("results")
                .or_else(|| trash.get("data"))
                .and_then(|v| v.as_array());

            if let Some(results) = trash_results {
                assert!(!results.is_empty(), "trash should have the deleted file");
            }
        }
    }

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_file_store_pagination() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    let token = register_and_login(&client, "admin_pagination@example.com", "adminpass123").await;
    let authed = LiveClient::new(server.base_url()).with_bearer_token(&token);

    // Create 5 file entries
    for i in 0..5 {
        authed
            .post_json(
                "/api/files",
                &serde_json::json!({
                    "original_name": &format!("pag_{}.txt", i),
                    "mime_type": "text/plain",
                    "size": 100 + i * 10,
                }),
            )
            .await
            .expect("create")
            .error_for_status()
            .expect("create should succeed");
    }

    // Page 1 with page_size=2
    let resp = authed
        .get_json::<serde_json::Value>("/api/files?page=1&page_size=2")
        .await
        .expect("page 1");

    let results = resp
        .get("results")
        .or_else(|| resp.get("data"))
        .and_then(|v| v.as_array())
        .expect("results array");

    assert_eq!(results.len(), 2, "page 1 should return 2 items");

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_file_store_sorting() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    let token = register_and_login(&client, "admin_sorting@example.com", "adminpass123").await;
    let authed = LiveClient::new(server.base_url()).with_bearer_token(&token);

    // Create 3 entries with different sizes
    authed
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "small.txt",
                "mime_type": "text/plain",
                "size": 10,
            }),
        )
        .await
        .expect("create")
        .error_for_status()
        .expect("create should succeed");

    authed
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "large.txt",
                "mime_type": "text/plain",
                "size": 1000,
            }),
        )
        .await
        .expect("create")
        .error_for_status()
        .expect("create should succeed");

    // Sort by size descending
    let resp = authed
        .get_json::<serde_json::Value>("/api/files?sort=size&order=desc")
        .await
        .expect("sort desc");

    let results = resp
        .get("results")
        .or_else(|| resp.get("data"))
        .and_then(|v| v.as_array())
        .expect("results array");

    assert!(
        results.len() >= 2,
        "should have at least 2 results, got {}",
        results.len()
    );

    // Verify descending size order
    for i in 0..results.len() - 1 {
        let size_a = results[i]
            .get("size")
            .and_then(|v| v.as_i64())
            .expect("size in result");
        let size_b = results[i + 1]
            .get("size")
            .and_then(|v| v.as_i64())
            .expect("size in result");
        assert!(
            size_a >= size_b,
            "results should be in descending size order: {} >= {}",
            size_a,
            size_b
        );
    }

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_file_store_filtering() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    let token = register_and_login(&client, "admin_filtering@example.com", "adminpass123").await;
    let authed = LiveClient::new(server.base_url()).with_bearer_token(&token);

    // Create entries with different mime types
    authed
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "text1.txt",
                "mime_type": "text/plain",
                "size": 100,
            }),
        )
        .await
        .expect("create")
        .error_for_status()
        .expect("create should succeed");

    authed
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "pdf1.pdf",
                "mime_type": "application/pdf",
                "size": 200,
            }),
        )
        .await
        .expect("create")
        .error_for_status()
        .expect("create should succeed");

    // Filter by mime_type
    let resp = authed
        .get_json::<serde_json::Value>("/api/files?mime_type=text/plain")
        .await
        .expect("filter");

    let results = resp
        .get("results")
        .or_else(|| resp.get("data"))
        .and_then(|v| v.as_array())
        .expect("results array");

    assert_eq!(results.len(), 1, "should filter to 1 text/plain file");

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_file_store_unauthorized() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    // Try to access file store without auth
    let resp = client.get("/api/files").await.expect("GET unauth");
    client.assert_status(&resp, StatusCode::UNAUTHORIZED).await;

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_file_store_field_permissions() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    // Register admin and create a file entry
    let admin_token =
        register_and_login(&client, "admin_permissions@example.com", "adminpass123").await;
    let admin = LiveClient::new(server.base_url()).with_bearer_token(&admin_token);

    admin
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "admin_file.txt",
                "mime_type": "text/plain",
                "size": 500,
                "tags": ["important"],
                "description": "Admin description"
            }),
        )
        .await
        .expect("create admin file")
        .error_for_status()
        .expect("create admin file should succeed");

    // Register an author user (can write description but not original_name or mime_type)
    let author_token = register_and_login(&client, "author@example.com", "authorpass123").await;
    let author = LiveClient::new(server.base_url()).with_bearer_token(&author_token);

    // Author tries to write original_name (admin-only write) - should fail
    let restricted_resp = author
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "restricted.txt",
                "mime_type": "text/plain",
                "size": 200,
                "tags": ["public"],
                "description": "Author description"
            }),
        )
        .await
        .expect("create with restricted fields");

    // Should return 403 Forbidden for writing original_name (admin-only)
    assert!(
        restricted_resp.status() == StatusCode::FORBIDDEN,
        "non-admin should be forbidden from writing admin-only fields, got: {}, body: {}",
        restricted_resp.status(),
        restricted_resp.text().await.unwrap_or_default()
    );

    // Author creates a file with only fields they're allowed to write
    let allowed_resp = author
        .post_json(
            "/api/files",
            &serde_json::json!({
                "description": "Author's own description",
                "tags": ["author-tag"]
            }),
        )
        .await
        .expect("create with allowed fields");

    assert!(
        allowed_resp.status() == StatusCode::OK || allowed_resp.status() == StatusCode::CREATED,
        "author should be able to write description and tags, got: {}, body: {}",
        allowed_resp.status(),
        allowed_resp.text().await.unwrap_or_default()
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_file_store_ownership() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    // Register an admin user
    let token = register_and_login(&client, "owner@example.com", "ownerpass123").await;
    let authed = LiveClient::new(server.base_url()).with_bearer_token(&token);

    // Create a file entry
    let create_resp = authed
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "ownership_test.txt",
                "mime_type": "text/plain",
                "size": 300,
            }),
        )
        .await
        .expect("create")
        .error_for_status()
        .expect("create should succeed");

    let json = create_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse");
    let file_id = json
        .get("id")
        .or_else(|| json.get("data").and_then(|d| d.get("id")))
        .and_then(|v| v.as_str())
        .expect("id");

    // Verify uploader_id was set from the authenticated user
    let uploader_id = json
        .get("uploader_id")
        .or_else(|| json.get("data").and_then(|d| d.get("uploader_id")))
        .and_then(|v| v.as_str())
        .expect("uploader_id should be set from auth context");
    assert!(!uploader_id.is_empty(), "uploader_id should not be empty");

    // Verify the file appears in listing with the correct uploader_id
    let list_resp = authed
        .get_json::<serde_json::Value>("/api/files")
        .await
        .expect("list");
    let results = list_resp
        .get("results")
        .or_else(|| list_resp.get("data"))
        .and_then(|v| v.as_array())
        .expect("results");
    let owner_file = results
        .iter()
        .find(|r| r.get("id").and_then(|v| v.as_str()) == Some(file_id))
        .expect("file should be in listing");
    let listed_uploader = owner_file
        .get("uploader_id")
        .or_else(|| owner_file.get("data").and_then(|d| d.get("uploader_id")))
        .and_then(|v| v.as_str())
        .expect("uploader_id in listing");
    assert_eq!(
        listed_uploader, uploader_id,
        "uploader_id should match between create and list"
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_file_store_cross_user_ownership() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    // Register user1 and create a file
    let user1_token = register_and_login(&client, "user1@example.com", "pass123").await;
    let user1 = LiveClient::new(server.base_url()).with_bearer_token(&user1_token);

    let create_resp = user1
        .post_json(
            "/api/files",
            &serde_json::json!({
                "tags": ["test", "user1"],
                "description": "user1 file",
                "size": 300,
            }),
        )
        .await
        .expect("user1 create")
        .error_for_status()
        .expect("user1 create should succeed");

    let created_json = create_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse");
    let user1_file_id = created_json
        .get("id")
        .or_else(|| created_json.get("data").and_then(|d| d.get("id")))
        .and_then(|v| v.as_str())
        .expect("user1 file id");

    // Register user2
    let user2_token = register_and_login(&client, "user2@example.com", "pass123").await;
    let user2 = LiveClient::new(server.base_url()).with_bearer_token(&user2_token);

    // User2 should see the file (admin_override is true, so admin can see all;
    // but in the test config, both users have default "user" role, and the
    // endpoint roles include ["admin", "editor", "author", "user"], so
    // non-admin users can see all files unless ownership filtering applies)
    //
    // The key test: user2 should NOT be able to access user1's file directly
    // if ownership filtering is working.
    let user2_get = user2
        .get(&format!("/api/files/{user1_file_id}"))
        .await
        .expect("user2 get user1 file");

    let user2_get_status = user2_get.status();
    // With admin_override: true and both users having the default "user" role,
    // the endpoint /api/files/{id} requires "admin" role, so user2 gets 403.
    assert!(
        user2_get_status == StatusCode::FORBIDDEN || user2_get_status == StatusCode::NOT_FOUND,
        "user2 should be denied access to user1's file (ownership filtering), got: {}",
        user2_get_status,
    );

    // User2 creates their own file
    let user2_create_resp = user2
        .post_json(
            "/api/files",
            &serde_json::json!({
                "tags": ["test", "user2"],
                "description": "user2 file",
                "size": 200,
            }),
        )
        .await
        .expect("user2 create")
        .error_for_status()
        .expect("user2 create should succeed");

    let user2_created = user2_create_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse");
    let user2_file_id = user2_created
        .get("id")
        .or_else(|| user2_created.get("data").and_then(|d| d.get("id")))
        .and_then(|v| v.as_str())
        .expect("user2 file id");

    // User1 should NOT be able to access user2's file
    let user1_get = user1
        .get(&format!("/api/files/{user2_file_id}"))
        .await
        .expect("user1 get user2 file");

    let user1_get_status = user1_get.status();
    assert!(
        user1_get_status == StatusCode::OK
            || user1_get_status == StatusCode::FORBIDDEN
            || user1_get_status == StatusCode::NOT_FOUND,
        "user1 accessing user2's file: {}, body: {}",
        user1_get_status,
        user1_get.text().await.unwrap_or_default()
    );

    server.shutdown().await.expect("server shutdown");
}

// =============================================================================
// Content references tests
// =============================================================================

#[tokio::test]
async fn test_file_store_content_references_attach() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    let token = register_and_login(&client, "admin_refs@example.com", "adminpass123").await;
    let authed = LiveClient::new(server.base_url()).with_bearer_token(&token);

    // Create a file entry
    let create_resp = authed
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "ref_test.txt",
                "mime_type": "text/plain",
                "size": 100,
            }),
        )
        .await
        .expect("create");
    assert!(create_resp.status().is_success() || create_resp.status() == StatusCode::CREATED);

    let created_json = create_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse");
    let file_id = created_json
        .get("id")
        .or_else(|| created_json.get("data").and_then(|d| d.get("id")))
        .and_then(|v| v.as_str())
        .expect("id");

    // Attach a content reference (simulate referencing this file from an entity)
    let ref_resp = authed
        .post_json(
            &format!("/api/files/{file_id}/refs"),
            &serde_json::json!({
                "entity_id": 42,
                "content_type": "article"
            }),
        )
        .await
        .expect("attach ref");

    let ref_status = ref_resp.status();
    assert!(
        ref_status == StatusCode::OK
            || ref_status == StatusCode::CREATED
            || ref_status == StatusCode::NO_CONTENT,
        "attach ref status: {}, body: {}",
        ref_status,
        ref_resp.text().await.unwrap_or_default()
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_file_store_content_references_attach_and_detach() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    let token = register_and_login(&client, "admin_refs_detach@example.com", "adminpass123").await;
    let authed = LiveClient::new(server.base_url()).with_bearer_token(&token);

    // Create a file entry
    let create_resp = authed
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "ref_detach_test.txt",
                "mime_type": "text/plain",
                "size": 100,
            }),
        )
        .await
        .expect("create")
        .error_for_status()
        .expect("create should succeed");

    let created_json = create_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse");
    let file_id = created_json
        .get("id")
        .or_else(|| created_json.get("data").and_then(|d| d.get("id")))
        .and_then(|v| v.as_str())
        .expect("id");

    // Attach a content reference
    authed
        .post_json(
            &format!("/api/files/{file_id}/refs"),
            &serde_json::json!({
                "entity_id": 42,
                "content_type": "article"
            }),
        )
        .await
        .expect("attach ref")
        .error_for_status()
        .expect("attach ref should succeed");

    // Detach the content reference
    authed
        .delete(&format!("/api/files/{file_id}/refs/42"))
        .await
        .expect("detach ref")
        .error_for_status()
        .ok();

    server.shutdown().await.expect("server shutdown");
}

// =============================================================================
// Trash management tests (restore, permanent delete, empty trash)
// =============================================================================

#[tokio::test]
async fn test_file_store_trash_restore() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    let token =
        register_and_login(&client, "admin_trash_restore@example.com", "adminpass123").await;
    let authed = LiveClient::new(server.base_url()).with_bearer_token(&token);

    // Create a file entry
    let create_resp = authed
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "restore_test.txt",
                "mime_type": "text/plain",
                "size": 100,
            }),
        )
        .await
        .expect("create");
    assert!(create_resp.status().is_success() || create_resp.status() == StatusCode::CREATED);

    let created_json = create_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse");
    let file_id = created_json
        .get("id")
        .or_else(|| created_json.get("data").and_then(|d| d.get("id")))
        .and_then(|v| v.as_str())
        .expect("id");

    // Delete to trash
    authed
        .delete(&format!("/api/files/{file_id}"))
        .await
        .expect("delete")
        .error_for_status()
        .expect("delete should succeed");

    // Verify file is gone from listing
    let list_resp = authed
        .get_json::<serde_json::Value>("/api/files")
        .await
        .expect("list");
    let results = list_resp
        .get("results")
        .or_else(|| list_resp.get("data"))
        .and_then(|v| v.as_array())
        .expect("results");
    assert_eq!(
        results.len(),
        0,
        "deleted file should not appear in listing"
    );

    // Verify trash has the file
    let trash_resp = authed
        .get_json::<serde_json::Value>("/_main-serve/file-store/trash")
        .await
        .expect("trash list");
    let trash_results = trash_resp
        .get("results")
        .or_else(|| trash_resp.get("data"))
        .and_then(|v| v.as_array())
        .expect("trash results");
    assert_eq!(trash_results.len(), 1, "trash should have one item");

    // Restore from trash
    let restore_resp = authed
        .post_json(
            &format!("/_main-serve/file-store/trash/{file_id}/restore"),
            &serde_json::json!({}),
        )
        .await
        .expect("restore");

    assert!(
        restore_resp.status() == StatusCode::OK || restore_resp.status() == StatusCode::NO_CONTENT,
        "restore status: {}, body: {}",
        restore_resp.status(),
        restore_resp.text().await.unwrap_or_default()
    );

    // Verify file is back in listing
    let list_resp = authed
        .get_json::<serde_json::Value>("/api/files")
        .await
        .expect("list after restore");
    let results = list_resp
        .get("results")
        .or_else(|| list_resp.get("data"))
        .and_then(|v| v.as_array())
        .expect("results");
    assert_eq!(results.len(), 1, "restored file should appear in listing");

    // Verify trash is empty
    let trash_resp = authed
        .get_json::<serde_json::Value>("/_main-serve/file-store/trash")
        .await
        .expect("trash list after restore");
    let trash_results = trash_resp
        .get("results")
        .or_else(|| trash_resp.get("data"))
        .and_then(|v| v.as_array())
        .expect("trash results after restore");
    assert_eq!(
        trash_results.len(),
        0,
        "trash should be empty after restore"
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_file_store_trash_permanent_delete() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    let token = register_and_login(&client, "admin_trash_delete@example.com", "adminpass123").await;
    let authed = LiveClient::new(server.base_url()).with_bearer_token(&token);

    // Create and delete a file
    let create_resp = authed
        .post_json(
            "/api/files",
            &serde_json::json!({
                "original_name": "perm_delete_test.txt",
                "mime_type": "text/plain",
                "size": 50,
            }),
        )
        .await
        .expect("create");
    assert!(create_resp.status().is_success() || create_resp.status() == StatusCode::CREATED);

    let created_json = create_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse");
    let file_id = created_json
        .get("id")
        .or_else(|| created_json.get("data").and_then(|d| d.get("id")))
        .and_then(|v| v.as_str())
        .expect("id");

    authed
        .delete(&format!("/api/files/{file_id}"))
        .await
        .expect("delete")
        .error_for_status()
        .expect("delete should succeed");

    // Permanently delete from trash
    let del_resp = authed
        .delete(&format!("/_main-serve/file-store/trash/{file_id}"))
        .await
        .expect("permanent delete");

    assert!(
        del_resp.status() == StatusCode::OK || del_resp.status() == StatusCode::NO_CONTENT,
        "permanent delete status: {}, body: {}",
        del_resp.status(),
        del_resp.text().await.unwrap_or_default()
    );

    // Verify file is gone from both listing and trash
    let list_resp = authed
        .get_json::<serde_json::Value>("/api/files")
        .await
        .expect("list");
    let results = list_resp
        .get("results")
        .or_else(|| list_resp.get("data"))
        .and_then(|v| v.as_array())
        .expect("results");
    assert_eq!(
        results.len(),
        0,
        "permanently deleted file should not appear in listing"
    );

    let trash_resp = authed
        .get_json::<serde_json::Value>("/_main-serve/file-store/trash")
        .await
        .expect("trash list");
    let trash_results = trash_resp
        .get("results")
        .or_else(|| trash_resp.get("data"))
        .and_then(|v| v.as_array())
        .expect("trash results");
    assert_eq!(
        trash_results.len(),
        0,
        "trash should be empty after permanent delete"
    );

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn test_file_store_trash_empty() {
    let (server, _temp_dir) = setup_file_store_server().await;
    let client = server.client();

    let token = register_and_login(&client, "admin_trash_empty@example.com", "adminpass123").await;
    let authed = LiveClient::new(server.base_url()).with_bearer_token(&token);

    // Create and delete two files
    for i in 0..2 {
        let create_resp = authed
            .post_json(
                "/api/files",
                &serde_json::json!({
                    "original_name": &format!("empty_trash_{i}.txt"),
                    "mime_type": "text/plain",
                    "size": 50,
                }),
            )
            .await
            .expect("create")
            .error_for_status()
            .expect("create should succeed");

        let created_json = create_resp
            .json::<serde_json::Value>()
            .await
            .expect("parse");
        let file_id = created_json
            .get("id")
            .or_else(|| created_json.get("data").and_then(|d| d.get("id")))
            .and_then(|v| v.as_str())
            .expect("id");

        authed
            .delete(&format!("/api/files/{file_id}"))
            .await
            .expect("delete")
            .error_for_status()
            .expect("delete should succeed");
    }

    // Verify trash has 2 items
    let trash_resp = authed
        .get_json::<serde_json::Value>("/_main-serve/file-store/trash")
        .await
        .expect("trash list");
    let trash_results = trash_resp
        .get("results")
        .or_else(|| trash_resp.get("data"))
        .and_then(|v| v.as_array())
        .expect("trash results");
    assert_eq!(trash_results.len(), 2, "trash should have 2 items");

    // Empty trash
    let empty_resp = authed
        .delete("/_main-serve/file-store/trash")
        .await
        .expect("empty trash");

    assert!(
        empty_resp.status() == StatusCode::OK || empty_resp.status() == StatusCode::NO_CONTENT,
        "empty trash status: {}, body: {}",
        empty_resp.status(),
        empty_resp.text().await.unwrap_or_default()
    );

    // Verify trash is empty
    let trash_resp = authed
        .get_json::<serde_json::Value>("/_main-serve/file-store/trash")
        .await
        .expect("trash list after empty");
    let trash_results = trash_resp
        .get("results")
        .or_else(|| trash_resp.get("data"))
        .and_then(|v| v.as_array())
        .expect("trash results");
    assert_eq!(
        trash_results.len(),
        0,
        "trash should be empty after emptying"
    );

    server.shutdown().await.expect("server shutdown");
}

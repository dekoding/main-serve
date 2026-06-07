/// Integration tests for the storage backend abstraction.
///
/// Tests store factory, `AppState` store lookups, and backend behavior.
mod support;

use axum::body::Body;
use axum::http::Request;
use main_serve::config::types::{StoreBackend, StoreConfig};
use main_serve::server::AppState;
use std::path::PathBuf;
use support::helpers::load_yaml;
use tower::ServiceExt;

#[allow(clippy::to_string_in_format_args)]
fn root_str(dir: &tempfile::TempDir) -> String {
    dir.path().display().to_string()
}

// =============================================================================
// Store factory tests (unit-level via AppState construction)
// =============================================================================

#[tokio::test]
async fn test_create_native_store_from_config() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let yaml = format!(
        r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/static/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "assets"
    auth: "none"
"#,
        root = root_str(&dir)
    );
    let config = load_yaml(&yaml).unwrap();
    assert!(config.stores.contains_key("assets"));
    let store = config.stores.get("assets").unwrap();
    assert_eq!(store.backend, StoreBackend::Native);
    assert_eq!(
        store.root.as_deref(),
        Some(dir.path().display().to_string().as_str())
    );
}

#[tokio::test]
async fn test_create_memory_store_from_config() {
    let yaml = r#"
server:
  port: 0

stores:
  mem_store:
    backend: memory

endpoints:
  - path: "/files/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "mem_store"
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    assert!(config.stores.contains_key("mem_store"));
    let store = config.stores.get("mem_store").unwrap();
    assert_eq!(store.backend, StoreBackend::Memory);
    assert!(store.root.is_none());
}

#[tokio::test]
async fn test_app_state_default_store_when_no_stores_defined() {
    let yaml = r#"
server:
  port: 0

endpoints:
  - path: "/health"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      body: "{\"status\":\"ok\"}"
      content_type: "application/json"
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    assert!(config.stores.is_empty());

    let _state = AppState::new(config, PathBuf::from("/dev/null"), "test-token".to_string())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_app_state_stores_with_multiple_backends() {
    let dir1 = tempfile::TempDir::new().expect("tempdir");
    let dir2 = tempfile::TempDir::new().expect("tempdir");
    let yaml = format!(
        r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "{root1}"
  data:
    backend: native
    root: "{root2}"
  temp:
    backend: memory

endpoints:
  - path: "/health"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      body: "{{\"status\":\"ok\"}}"
      content_type: "application/json"
    auth: "none"
"#,
        root1 = dir1.path().display(),
        root2 = dir2.path().display(),
    );
    let config = load_yaml(&yaml).unwrap();
    assert_eq!(config.stores.len(), 3);
    assert!(config.stores.contains_key("assets"));
    assert!(config.stores.contains_key("data"));
    assert!(config.stores.contains_key("temp"));
    let _state = AppState::new(config, PathBuf::from("/dev/null"), "test-token".to_string())
        .await
        .unwrap();
}

// =============================================================================
// Store lookup in AppState via integration test
// =============================================================================

#[tokio::test]
async fn test_app_state_store_lookup() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let yaml = format!(
        r#"
server:
  port: 0

stores:
  test_store:
    backend: native
    root: "{root}"

endpoints:
  - path: "/health"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      body: "{{\"status\":\"ok\"}}"
      content_type: "application/json"
    auth: "none"
"#,
        root = dir.path().display()
    );
    let config = load_yaml(&yaml).unwrap();
    let state = AppState::new(config, PathBuf::from("/dev/null"), "test-token".to_string())
        .await
        .unwrap();
    assert!(state.get_store("test_store").is_some());
    assert!(state.get_store("nonexistent").is_none());
}

#[tokio::test]
async fn test_app_state_get_store_returns_none_for_unknown_name() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let yaml = format!(
        r#"
server:
  port: 0

stores:
  only_store:
    backend: native
    root: "{root}"

endpoints:
  - path: "/health"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      body: "{{\"status\":\"ok\"}}"
      content_type: "application/json"
    auth: "none"
"#,
        root = dir.path().display()
    );
    let config = load_yaml(&yaml).unwrap();
    let state = AppState::new(config, PathBuf::from("/dev/null"), "test-token".to_string())
        .await
        .unwrap();
    assert!(state.get_store("only_store").is_some());
    assert!(state.get_store("other").is_none());
    assert!(state.get_store("").is_none());
}

// =============================================================================
// Storage backend unit behavior tests (NativeStorage via create_store)
// =============================================================================

#[tokio::test]
async fn test_native_store_write_and_read() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = StoreConfig {
        backend: StoreBackend::Native,
        root: Some(dir.path().to_str().unwrap().to_string()),
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    // Write a file
    store
        .write(PathBuf::from("test.txt").as_path(), b"hello world")
        .await
        .unwrap();

    // Read it back
    let contents = store
        .read(PathBuf::from("test.txt").as_path())
        .await
        .unwrap();
    assert_eq!(contents, b"hello world");
}

#[tokio::test]
async fn test_native_store_exists_and_is_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = StoreConfig {
        backend: StoreBackend::Native,
        root: Some(dir.path().to_str().unwrap().to_string()),
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    // Non-existent path
    assert!(!store.exists(PathBuf::from("missing.txt").as_path()).await);
    assert!(!store.is_file(PathBuf::from("missing.txt").as_path()).await);

    // Create file
    store
        .write(PathBuf::from("exists.txt").as_path(), b"content")
        .await
        .unwrap();

    assert!(store.exists(PathBuf::from("exists.txt").as_path()).await);
    assert!(store.is_file(PathBuf::from("exists.txt").as_path()).await);
}

#[tokio::test]
async fn test_native_store_create_dir_and_list() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = StoreConfig {
        backend: StoreBackend::Native,
        root: Some(dir.path().to_str().unwrap().to_string()),
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    // Create directory structure
    store
        .create_dir_all(PathBuf::from("a/b/c").as_path())
        .await
        .unwrap();

    assert!(store.is_dir(PathBuf::from("a").as_path()).await);
    assert!(store.is_dir(PathBuf::from("a/b").as_path()).await);
    assert!(store.is_dir(PathBuf::from("a/b/c").as_path()).await);

    // Write files in the directory
    store
        .write(PathBuf::from("a/b/file1.txt").as_path(), b"one")
        .await
        .unwrap();
    store
        .write(PathBuf::from("a/b/file2.txt").as_path(), b"two")
        .await
        .unwrap();

    // List directory (2 files + 1 subdirectory = 3 entries)
    let entries = store.list(PathBuf::from("a/b").as_path()).await.unwrap();
    assert_eq!(entries.len(), 3);
}

#[tokio::test]
async fn test_native_store_delete_and_rename() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = StoreConfig {
        backend: StoreBackend::Native,
        root: Some(dir.path().to_str().unwrap().to_string()),
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    // Write file
    store
        .write(PathBuf::from("old.txt").as_path(), b"data")
        .await
        .unwrap();
    assert!(store.exists(PathBuf::from("old.txt").as_path()).await);

    // Rename
    store
        .rename(
            PathBuf::from("old.txt").as_path(),
            PathBuf::from("new.txt").as_path(),
        )
        .await
        .unwrap();
    assert!(!store.exists(PathBuf::from("old.txt").as_path()).await);
    assert!(store.exists(PathBuf::from("new.txt").as_path()).await);

    // Delete
    store
        .delete(PathBuf::from("new.txt").as_path())
        .await
        .unwrap();
    assert!(!store.exists(PathBuf::from("new.txt").as_path()).await);
}

#[tokio::test]
async fn test_native_store_copy() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = StoreConfig {
        backend: StoreBackend::Native,
        root: Some(dir.path().to_str().unwrap().to_string()),
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    // Write source
    store
        .write(PathBuf::from("src.txt").as_path(), b"original")
        .await
        .unwrap();

    // Copy
    store
        .copy(
            PathBuf::from("src.txt").as_path(),
            PathBuf::from("dst.txt").as_path(),
        )
        .await
        .unwrap();

    // Both exist with same content
    let src = store
        .read(PathBuf::from("src.txt").as_path())
        .await
        .unwrap();
    let dst = store
        .read(PathBuf::from("dst.txt").as_path())
        .await
        .unwrap();
    assert_eq!(src, dst);
    assert_eq!(src, b"original");
}

#[tokio::test]
async fn test_native_store_metadata_and_size() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = StoreConfig {
        backend: StoreBackend::Native,
        root: Some(dir.path().to_str().unwrap().to_string()),
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    let content = b"hello world";
    store
        .write(PathBuf::from("test.dat").as_path(), content)
        .await
        .unwrap();

    let meta = store
        .metadata(PathBuf::from("test.dat").as_path())
        .await
        .unwrap();
    assert!(meta.is_file);

    let size = store
        .size(PathBuf::from("test.dat").as_path())
        .await
        .unwrap();
    assert_eq!(size, content.len() as u64);
}

// =============================================================================
// MemoryStorage tests
// =============================================================================

#[tokio::test]
async fn test_memory_store_write_and_read() {
    let config = StoreConfig {
        backend: StoreBackend::Memory,
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    store
        .write(PathBuf::from("memory.txt").as_path(), b"mem content")
        .await
        .unwrap();

    let contents = store
        .read(PathBuf::from("memory.txt").as_path())
        .await
        .unwrap();
    assert_eq!(contents, b"mem content");
}

#[tokio::test]
async fn test_memory_store_isolation() {
    // Each MemoryStorage instance should be isolated
    let config1 = StoreConfig {
        backend: StoreBackend::Memory,
        ..Default::default()
    };
    let store1 = main_serve::storage::create_store(&config1).await.unwrap();

    let config2 = StoreConfig {
        backend: StoreBackend::Memory,
        ..Default::default()
    };
    let store2 = main_serve::storage::create_store(&config2).await.unwrap();

    store1
        .write(PathBuf::from("shared.txt").as_path(), b"from store1")
        .await
        .unwrap();

    // Store2 should not see store1's files
    assert!(
        store2
            .read(PathBuf::from("shared.txt").as_path())
            .await
            .is_err()
    );

    // Store1 should still have its file
    let contents = store1
        .read(PathBuf::from("shared.txt").as_path())
        .await
        .unwrap();
    assert_eq!(contents, b"from store1");
}

#[tokio::test]
async fn test_memory_store_exists_and_list() {
    let config = StoreConfig {
        backend: StoreBackend::Memory,
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    // Empty store
    assert!(!store.exists(PathBuf::from("anything").as_path()).await);
    let entries = store.list(PathBuf::from("").as_path()).await.unwrap();
    assert!(entries.is_empty());

    // Write some files
    store
        .write(PathBuf::from("a.txt").as_path(), b"a")
        .await
        .unwrap();
    store
        .write(PathBuf::from("b.txt").as_path(), b"b")
        .await
        .unwrap();

    assert!(store.exists(PathBuf::from("a.txt").as_path()).await);
    assert!(store.is_file(PathBuf::from("a.txt").as_path()).await);

    let entries = store.list(PathBuf::from("").as_path()).await.unwrap();
    assert_eq!(entries.len(), 2);
}

// =============================================================================
// NativeStorage root_path() test
// =============================================================================

#[tokio::test]
async fn test_native_store_root_path() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = StoreConfig {
        backend: StoreBackend::Native,
        root: Some(dir.path().to_str().unwrap().to_string()),
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    let root = store.root_path();
    assert!(root.is_some());
    let root_path = root.unwrap();
    // The root should be the configured directory
    assert!(root_path.is_absolute() || root_path.starts_with("."));
}

#[tokio::test]
async fn test_memory_store_root_path_none() {
    let config = StoreConfig {
        backend: StoreBackend::Memory,
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    // Memory storage should not report a root path
    assert!(store.root_path().is_none());
}

// =============================================================================
// SPA host with valid store returns 404 for missing files
// =============================================================================

#[tokio::test]
async fn test_spa_host_missing_file_returns_404() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let yaml = format!(
        r#"
server:
  port: 0

stores:
  assets:
    backend: native
    root: "{root}"

endpoints:
  - path: "/files/*"
    methods: ["get"]
    action: "spa_host"
    spa_host:
      storage: "assets"
      index: "index.html"
    auth: "none"
"#,
        root = dir.path().display()
    );
    let (app, _f) = support::setup_server(&yaml).await;

    let req = Request::builder()
        .uri("/files/index.html")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

// =============================================================================
// 5.1 Store integration tests - memory store under load
// =============================================================================

#[tokio::test]
async fn test_memory_store_concurrent_writes() {
    let config = StoreConfig {
        backend: StoreBackend::Memory,
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    let mut handles = Vec::new();
    for i in 0..50 {
        let s = store.clone();
        handles.push(tokio::spawn(async move {
            let path = PathBuf::from(format!("concurrent_{}.txt", i));
            s.write(&path, format!("content-{}", i).as_bytes())
                .await
                .unwrap();
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }

    for i in 0..50 {
        let path = PathBuf::from(format!("concurrent_{}.txt", i));
        let contents = store.read(&path).await.unwrap();
        assert_eq!(contents, format!("content-{}", i).as_bytes());
    }
}

#[tokio::test]
async fn test_memory_store_concurrent_read_write_mixed() {
    let config = StoreConfig {
        backend: StoreBackend::Memory,
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    // Pre-populate
    for i in 0..20 {
        store
            .write(
                PathBuf::from(format!("shared_{}.txt", i)).as_path(),
                format!("initial-{}", i).as_bytes(),
            )
            .await
            .unwrap();
    }

    let mut handles = Vec::new();
    for i in 0..20 {
        let s = store.clone();
        let i2 = i;
        handles.push(tokio::spawn(async move {
            let path = PathBuf::from(format!("shared_{}.txt", i2));
            // Read first
            let _ = s.read(&path).await;
            // Then write
            s.write(&path, format!("updated-{}", i2).as_bytes())
                .await
                .unwrap();
            // Read again
            let _ = s.read(&path).await;
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }

    for i in 0..20 {
        let path = PathBuf::from(format!("shared_{}.txt", i));
        let contents = store.read(&path).await.unwrap();
        assert_eq!(contents, format!("updated-{}", i).as_bytes());
    }
}

#[tokio::test]
async fn test_memory_store_stress_large_payloads() {
    let config = StoreConfig {
        backend: StoreBackend::Memory,
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    let mut handles = Vec::new();
    for i in 0..10 {
        let s = store.clone();
        let payload = vec![i as u8; 10_000];
        handles.push(tokio::spawn(async move {
            let path = PathBuf::from(format!("large_{}.bin", i));
            s.write(&path, &payload).await.unwrap();
            let read_back = s.read(&path).await.unwrap();
            assert_eq!(read_back.len(), 10_000);
            assert_eq!(&read_back, &payload);
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }
}

// =============================================================================
// 5.1 Store integration tests - native store file permission edge cases
// =============================================================================

#[tokio::test]
async fn test_native_store_read_nonexistent_returns_not_found() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = StoreConfig {
        backend: StoreBackend::Native,
        root: Some(dir.path().to_str().unwrap().to_string()),
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    let result = store.read(PathBuf::from("nonexistent.txt").as_path()).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        main_serve::storage::StorageError::Io(path, _) => {
            assert!(path.to_string_lossy().contains("nonexistent"));
        }
        other => panic!("expected Io error, got {:?}", other),
    }
}

#[tokio::test]
async fn test_native_store_write_creates_parent_dirs_not_auto() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = StoreConfig {
        backend: StoreBackend::Native,
        root: Some(dir.path().to_str().unwrap().to_string()),
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    // Writing to a nested path without creating parent dirs should work
    // because tokio::fs::write creates parent dirs automatically on most systems
    // Actually it does not - let's test the behavior
    let result = store
        .write(PathBuf::from("nested/deep/file.txt").as_path(), b"data")
        .await;
    // tokio::fs::write does NOT create parent directories
    assert!(result.is_err());
}

#[tokio::test]
async fn test_native_store_delete_nonexistent_returns_error() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = StoreConfig {
        backend: StoreBackend::Native,
        root: Some(dir.path().to_str().unwrap().to_string()),
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    let result = store
        .delete(PathBuf::from("nonexistent.txt").as_path())
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_native_store_write_read_binary_data() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = StoreConfig {
        backend: StoreBackend::Native,
        root: Some(dir.path().to_str().unwrap().to_string()),
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    let binary_data: Vec<u8> = (0..=255).collect();
    store
        .write(PathBuf::from("binary.dat").as_path(), &binary_data)
        .await
        .unwrap();

    let read_back = store
        .read(PathBuf::from("binary.dat").as_path())
        .await
        .unwrap();
    assert_eq!(read_back, binary_data);
}

#[tokio::test]
async fn test_native_store_list_nested_directories() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = StoreConfig {
        backend: StoreBackend::Native,
        root: Some(dir.path().to_str().unwrap().to_string()),
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    store.create_dir_all(&PathBuf::from("a/b/c")).await.unwrap();
    store
        .write(&PathBuf::from("a/b/c/file.txt"), b"deep")
        .await
        .unwrap();
    store
        .write(&PathBuf::from("a/file1.txt"), b"shallow")
        .await
        .unwrap();

    let entries_a = store.list(&PathBuf::from("a")).await.unwrap();
    assert_eq!(entries_a.len(), 2); // file1.txt + b/

    let entries_c = store.list(&PathBuf::from("a/b/c")).await.unwrap();
    assert_eq!(entries_c.len(), 1); // file.txt

    let deep_content = store.read(&PathBuf::from("a/b/c/file.txt")).await.unwrap();
    assert_eq!(deep_content, b"deep");
}

#[tokio::test]
async fn test_native_store_rename_across_subdirs() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = StoreConfig {
        backend: StoreBackend::Native,
        root: Some(dir.path().to_str().unwrap().to_string()),
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    store.create_dir_all(&PathBuf::from("src/d")).await.unwrap();
    store.create_dir_all(&PathBuf::from("dst")).await.unwrap();
    store
        .write(&PathBuf::from("src/d/old.txt"), b"move me")
        .await
        .unwrap();

    store
        .rename(
            &PathBuf::from("src/d/old.txt"),
            &PathBuf::from("dst/new.txt"),
        )
        .await
        .unwrap();

    assert!(!store.exists(&PathBuf::from("src/d/old.txt")).await);
    let content = store.read(&PathBuf::from("dst/new.txt")).await.unwrap();
    assert_eq!(content, b"move me");
}

#[tokio::test]
async fn test_native_store_copy_nonexistent_source_error() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = StoreConfig {
        backend: StoreBackend::Native,
        root: Some(dir.path().to_str().unwrap().to_string()),
        ..Default::default()
    };
    let store = main_serve::storage::create_store(&config).await.unwrap();

    let result = store
        .copy(
            PathBuf::from("nonexistent.txt").as_path(),
            PathBuf::from("dest.txt").as_path(),
        )
        .await;
    assert!(result.is_err());
}

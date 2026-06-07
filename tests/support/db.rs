use std::collections::HashMap;
use std::env;
use std::fmt;
use std::fs;
use std::net::ToSocketAddrs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::Router;
use tempfile::TempDir;

use main_serve::config::{AppConfig, load_config};
use main_serve::db::migration::run_migrations;
use main_serve::db::pool::{DatabasePool, create_pools};
use main_serve::server::{AppState, build_router};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TestBackend {
    Sqlite,
    Postgres,
    Mysql,
}

impl TestBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            TestBackend::Sqlite => "sqlite",
            TestBackend::Postgres => "postgres",
            TestBackend::Mysql => "mysql",
        }
    }

    pub fn placeholder(self, index: usize) -> String {
        match self {
            TestBackend::Postgres => format!("${index}"),
            TestBackend::Sqlite | TestBackend::Mysql => "?".to_string(),
        }
    }

    pub(crate) fn configured_url(self, root_dir: &TempDir, table_name: &str) -> String {
        match self {
            TestBackend::Sqlite => {
                let db_path = root_dir.path().join(format!("{table_name}.db"));
                format!("sqlite://{}?mode=rwc", db_path.display())
            }
            TestBackend::Postgres => env::var("TEST_POSTGRES_URL")
                .expect("TEST_POSTGRES_URL must be set when running postgres integration tests"),
            TestBackend::Mysql => env::var("TEST_MYSQL_URL")
                .expect("TEST_MYSQL_URL must be set when running mysql integration tests"),
        }
    }
}

impl fmt::Display for TestBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub fn enabled_backends() -> Vec<TestBackend> {
    if let Ok(value) = env::var("TEST_BACKENDS") {
        let mut backends = Vec::new();
        for name in value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            match name {
                "sqlite" => backends.push(TestBackend::Sqlite),
                "postgres" => backends.push(TestBackend::Postgres),
                "mysql" => backends.push(TestBackend::Mysql),
                other => panic!("Unsupported backend in TEST_BACKENDS: {other}"),
            }
        }
        return backends;
    }

    let mut backends = vec![TestBackend::Sqlite];

    if env::var("TEST_POSTGRES_URL")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
        || is_port_open("127.0.0.1:5432", Duration::from_millis(200))
    {
        backends.push(TestBackend::Postgres);
    }

    if env::var("TEST_MYSQL_URL")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
        || is_port_open("127.0.0.1:3306", Duration::from_millis(200))
    {
        backends.push(TestBackend::Mysql);
    }

    backends
}

/// Check if a TCP port is open by attempting a synchronous connection.
/// Works for localhost: connection refused is immediate, open ports succeed.
fn is_port_open(addr: &str, timeout: Duration) -> bool {
    let _ = timeout; // used by caller to decide whether to attempt check
    match addr
        .to_socket_addrs()
        .ok()
        .and_then(|mut iter| iter.next())
        .and_then(|a| std::net::TcpStream::connect(a).ok())
    {
        Some(stream) => {
            // Set a short read timeout so that if the connection
            // hangs (e.g. firewall drop), we don't block forever.
            let _ = stream.set_read_timeout(Some(timeout));
            let mut buf = [0u8; 1];
            stream.peek(&mut buf).is_ok()
        }
        None => false,
    }
}

pub struct TestDatabase {
    pub backend: TestBackend,
    pub table_name: String,
    pub db_url: String,
    pub root_dir: TempDir,
    pools: Option<HashMap<String, DatabasePool>>,
}

impl TestDatabase {
    pub fn new(backend: TestBackend, test_name: &str) -> Self {
        let root_dir = TempDir::new().expect("tempdir");
        let table_name = unique_identifier(test_name);
        let db_url = backend.configured_url(&root_dir, &table_name);

        Self {
            backend,
            table_name,
            db_url,
            root_dir,
            pools: None,
        }
    }

    pub fn render_yaml(&self, template: &str) -> String {
        template
            .replace("__DB_DRIVER__", self.backend.as_str())
            .replace("__DB_URL__", &self.db_url)
            .replace("__TABLE_NAME__", &self.table_name)
    }

    pub fn write_config(&self, template: &str, file_name: &str) -> PathBuf {
        let config_path = self.root_dir.path().join(file_name);
        let yaml = self.render_yaml(template);
        fs::write(&config_path, yaml).expect("write config");
        config_path
    }

    pub fn load_config(&self, template: &str, file_name: &str) -> AppConfig {
        let config_path = self.write_config(template, file_name);
        load_config(&config_path).expect("load config")
    }

    pub async fn setup_app(&mut self, template: &str, file_name: &str) -> (Router, AppState) {
        let config_path = self.write_config(template, file_name);
        let config = load_config(&config_path).expect("load config");
        let pools = create_pools_and_migrate(&config).await;

        let state = AppState::new(config, config_path, "test-token".to_string())
            .await
            .unwrap();
        {
            let mut pool_lock = state.db_pools.write().await;
            (*pool_lock).clone_from(&pools);
        }

        // Store a clone for Drop-based cleanup.
        self.pools = Some(pools);

        let config_guard = state.config.read().await;
        let app = build_router(&config_guard, state.clone()).await;
        drop(config_guard);

        (app, state)
    }

    /// Returns a reference to the database pools, if available.
    ///
    /// This is needed by tests that perform direct SQL queries after
    /// `setup_app` (e.g. migration tests). The pools are also cleaned
    /// up automatically when `TestDatabase` is dropped.
    pub fn db_pools(&self) -> Option<&HashMap<String, DatabasePool>> {
        self.pools.as_ref()
    }

    /// Store pools for Drop-based cleanup.
    ///
    /// Used by tests that create pools directly (not via `setup_app`)
    /// but still need table cleanup.
    pub fn set_pools(&mut self, pools: HashMap<String, DatabasePool>) {
        self.pools = Some(pools);
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        // Drop the test table from postgres/mysql databases.
        // SQLite cleanup is handled automatically by the TempDir.
        if self.backend == TestBackend::Sqlite {
            return;
        }
        if let Some(pools) = self.pools.take() {
            let table_name = self.table_name.clone();
            tokio::spawn(async move {
                if let Some(pool) = pools.get("main") {
                    let drop_sql = format!("DROP TABLE IF EXISTS {table_name}");
                    let _ = pool.execute_with_params(&drop_sql, &[]).await;
                }
            });
        }
    }
}

pub async fn create_pools_and_migrate(config: &AppConfig) -> HashMap<String, DatabasePool> {
    let pools = create_pools(&config.databases).await.expect("create pools");
    run_migrations(&config.tables, &pools, &config.databases)
        .await
        .expect("run migrations");
    pools
}

fn unique_identifier(test_name: &str) -> String {
    let counter = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut identifier: String = test_name
        .chars()
        .map(|ch| match ch {
            'a'..='z' | '0'..='9' => ch,
            'A'..='Z' => ch.to_ascii_lowercase(),
            _ => '_',
        })
        .collect();
    identifier.truncate(32);
    while identifier.contains("__") {
        identifier = identifier.replace("__", "_");
    }
    identifier = identifier.trim_matches('_').to_string();
    if identifier.is_empty() {
        identifier = "test_table".to_string();
    }
    format!("t_{identifier}_{counter}")
}

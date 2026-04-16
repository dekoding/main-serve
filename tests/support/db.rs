#![allow(dead_code)]

use std::collections::HashMap;
use std::env;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

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

    fn configured_url(self, root_dir: &TempDir, table_name: &str) -> String {
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
    {
        backends.push(TestBackend::Postgres);
    }
    if env::var("TEST_MYSQL_URL")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        backends.push(TestBackend::Mysql);
    }
    backends
}

pub struct TestDatabase {
    pub backend: TestBackend,
    pub table_name: String,
    pub db_url: String,
    root_dir: TempDir,
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

    pub async fn setup_app(&self, template: &str, file_name: &str) -> Router {
        let config_path = self.write_config(template, file_name);
        let config = load_config(&config_path).expect("load config");
        let pools = create_pools_and_migrate(&config).await;

        let state = AppState::new(config, config_path, "test-token".to_string());
        {
            let mut pool_lock = state.db_pools.write().await;
            *pool_lock = pools;
        }

        let config_guard = state.config.read().await;
        let app = build_router(&config_guard, state.clone());
        drop(config_guard);
        app
    }

    /// Drop the test table from postgres/mysql databases.
    ///
    /// SQLite cleanup is handled automatically by the `TempDir`. For
    /// postgres and mysql, tables with unique names accumulate across test
    /// runs. Call this at the end of tests that create persistent tables.
    pub async fn cleanup(&self, pools: &HashMap<String, DatabasePool>) {
        if self.backend == TestBackend::Sqlite {
            return;
        }
        if let Some(pool) = pools.get("main") {
            let drop_sql = format!("DROP TABLE IF EXISTS {}", self.table_name);
            let _ = pool.execute_with_params(&drop_sql, &[]).await;
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

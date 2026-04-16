mod support;

use main_serve::db::migration::run_migrations;
use main_serve::db::pool::{DatabasePool, create_pools};

use support::db::{TestDatabase, enabled_backends};

const ADD_COLUMN_V1: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: false

tables:
  __TABLE_NAME__:
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "title"
        type: "text"
        nullable: false

endpoints: []
"#;

const ADD_COLUMN_V2: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: false

tables:
  __TABLE_NAME__:
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "title"
        type: "text"
        nullable: false
      - name: "status"
        type: "varchar"
        nullable: false
        default: "'draft'"

endpoints: []
"#;

const DROP_COLUMN_BASE: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: __ALLOW_DESTRUCTIVE__

tables:
  __TABLE_NAME__:
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "title"
        type: "text"
        nullable: false
      - name: "body"
        type: "text"
        nullable: true

endpoints: []
"#;

const DROP_COLUMN_TARGET: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: __ALLOW_DESTRUCTIVE__

tables:
  __TABLE_NAME__:
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "title"
        type: "text"
        nullable: false

endpoints: []
"#;

const INDEX_CONFIG: &str = r#"
server:
  port: 0

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true
    allow_destructive: false

tables:
  __TABLE_NAME__:
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "email"
        type: "varchar"
        nullable: false
        indexed: true

endpoints: []
"#;

#[tokio::test]
async fn test_add_column_with_default_across_backends() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "migration_add_column");

        let v1_config = test_db.load_config(ADD_COLUMN_V1, "v1.yaml");
        let pools = create_pools(&v1_config.databases)
            .await
            .expect("create pools");
        run_migrations(&v1_config.tables, &pools, &v1_config.databases)
            .await
            .expect("run migrations v1");

        let pool = pools.get("main").unwrap();
        let insert_sql = format!(
            "INSERT INTO {} (title) VALUES ({})",
            test_db.table_name,
            backend.placeholder(1)
        );
        pool.execute_with_params(&insert_sql, &[serde_json::json!("hello")])
            .await
            .expect("insert row");

        let v2_config = test_db.load_config(ADD_COLUMN_V2, "v2.yaml");
        run_migrations(&v2_config.tables, &pools, &v2_config.databases)
            .await
            .expect("run migrations v2");

        let select_sql = format!(
            "SELECT title, status FROM {} ORDER BY id",
            test_db.table_name
        );
        let rows = pool
            .fetch_all_json(&select_sql, &[])
            .await
            .expect("fetch rows");

        assert_eq!(rows.len(), 1, "backend: {backend}");
        assert_eq!(rows[0]["title"], "hello", "backend: {backend}");
        assert_eq!(rows[0]["status"], "draft", "backend: {backend}");
    }
}

#[tokio::test]
async fn test_drop_column_requires_allow_destructive() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "migration_drop_column_safe");

        let base_template = DROP_COLUMN_BASE.replace("__ALLOW_DESTRUCTIVE__", "false");
        let target_template = DROP_COLUMN_TARGET.replace("__ALLOW_DESTRUCTIVE__", "false");

        let base_config = test_db.load_config(&base_template, "safe_v1.yaml");
        let pools = create_pools(&base_config.databases)
            .await
            .expect("create pools");
        run_migrations(&base_config.tables, &pools, &base_config.databases)
            .await
            .expect("run migrations safe v1");

        let target_config = test_db.load_config(&target_template, "safe_v2.yaml");
        run_migrations(&target_config.tables, &pools, &target_config.databases)
            .await
            .expect("run migrations safe v2");

        let select_sql = format!("SELECT body FROM {} LIMIT 1", test_db.table_name);
        let body_select = pools
            .get("main")
            .unwrap()
            .fetch_all_json(&select_sql, &[])
            .await;
        assert!(body_select.is_ok(), "backend: {backend}");
    }
}

#[tokio::test]
async fn test_drop_column_with_allow_destructive() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "migration_drop_column_destructive");

        let base_template = DROP_COLUMN_BASE.replace("__ALLOW_DESTRUCTIVE__", "true");
        let target_template = DROP_COLUMN_TARGET.replace("__ALLOW_DESTRUCTIVE__", "true");

        let base_config = test_db.load_config(&base_template, "destructive_v1.yaml");
        let pools = create_pools(&base_config.databases)
            .await
            .expect("create pools");
        run_migrations(&base_config.tables, &pools, &base_config.databases)
            .await
            .expect("run migrations destructive v1");

        let target_config = test_db.load_config(&target_template, "destructive_v2.yaml");
        run_migrations(&target_config.tables, &pools, &target_config.databases)
            .await
            .expect("run migrations destructive v2");

        let select_sql = format!("SELECT body FROM {} LIMIT 1", test_db.table_name);
        let body_select = pools
            .get("main")
            .unwrap()
            .fetch_all_json(&select_sql, &[])
            .await;
        assert!(body_select.is_err(), "backend: {backend}");
    }
}

#[tokio::test]
async fn test_index_creation_is_present_and_idempotent_across_backends() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "migration_indexes");
        let config = test_db.load_config(INDEX_CONFIG, "indexes.yaml");
        let pools = create_pools(&config.databases).await.expect("create pools");

        run_migrations(&config.tables, &pools, &config.databases)
            .await
            .expect("run migrations first pass");
        run_migrations(&config.tables, &pools, &config.databases)
            .await
            .expect("run migrations second pass");

        let indexes =
            get_index_names(pools.get("main").unwrap(), backend, &test_db.table_name).await;
        assert!(
            indexes.contains(&format!("idx_{}_email", test_db.table_name)),
            "backend: {backend}, indexes: {:?}",
            indexes
        );
    }
}

async fn get_index_names(
    pool: &DatabasePool,
    backend: support::db::TestBackend,
    table_name: &str,
) -> std::collections::HashSet<String> {
    let rows = match backend {
        support::db::TestBackend::Sqlite => {
          let sql = format!("PRAGMA index_list(\"{}\")", table_name);
          pool.fetch_all_json(&sql, &[]).await.expect("sqlite index list")
        }
        support::db::TestBackend::Postgres => pool
          .fetch_all_json(
            "SELECT indexname FROM pg_indexes WHERE tablename = $1",
            &[serde_json::Value::String(table_name.to_owned())],
          )
          .await
          .expect("postgres index list"),
        support::db::TestBackend::Mysql => pool
          .fetch_all_json(
            "SELECT index_name FROM information_schema.statistics WHERE table_schema = DATABASE() AND table_name = ? GROUP BY index_name",
            &[serde_json::Value::String(table_name.to_owned())],
          )
          .await
          .expect("mysql index list"),
      };

    rows.into_iter()
        .filter_map(|row| {
            row.get("name")
                .or_else(|| row.get("indexname"))
                .or_else(|| row.get("index_name"))
                .or_else(|| row.get("INDEX_NAME"))
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .collect()
}

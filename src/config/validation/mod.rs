mod auth;
mod databases;
mod endpoints;
mod server;
mod stores;
mod tables;

use super::types::AppConfig;

pub(crate) use auth::validate_auth;
pub(crate) use auth::validate_role_hierarchy;
pub(crate) use databases::validate_databases;
pub(crate) use endpoints::validate_endpoints;
pub(crate) use server::validate_server;
pub(crate) use stores::validate_stores;
pub(crate) use tables::validate_tables;

/// Validate a fully parsed `AppConfig` for semantic correctness.
///
/// Returns `Ok(())` if valid, or `Err(AppError::Validation(...))` with a
/// combined list of all problems found.
///
/// # Errors
///
/// Returns `AppError::Validation` containing all validation errors joined
/// into a single message.
#[must_use = "Result ignored"]
pub fn validate_config(config: &AppConfig) -> Result<(), AppError> {
    let mut errors: Vec<String> = Vec::new();

    validate_server(&config.server, &mut errors);
    validate_databases(config, &mut errors);
    validate_stores(&config.stores, &mut errors);
    validate_tables(config, &mut errors);
    validate_endpoints(config, &mut errors);
    validate_auth(config, &mut errors);
    validate_role_hierarchy(config.role_hierarchy.as_ref(), &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "Configuration validation failed:\n  - {}",
            errors.join("\n  - ")
        )))
    }
}

use crate::error::AppError;

#[cfg(test)]
mod tests {
    use super::super::types;
    use super::*;
    use std::collections::HashMap;

    use crate::config::types::{StoreBackend, StoreConfig};
    use types::RolesConfig;

    fn make_native_store(root: Option<&str>) -> StoreConfig {
        StoreConfig {
            backend: StoreBackend::Native,
            root: root.map(String::from),
            s3: None,
            azure: None,
            gcs: None,
        }
    }

    fn make_s3_store(
        region: Option<&str>,
        bucket: Option<&str>,
        access_key: Option<&str>,
        secret_key: Option<&str>,
    ) -> StoreConfig {
        StoreConfig {
            backend: StoreBackend::S3,
            root: None,
            s3: Some(types::S3StoreConfig {
                region: region.unwrap_or("").to_string(),
                bucket: bucket.unwrap_or("").to_string(),
                access_key: access_key.unwrap_or("").to_string(),
                secret_key: secret_key.unwrap_or("").to_string(),
                endpoint: None,
                force_path_style: false,
            }),
            azure: None,
            gcs: None,
        }
    }

    fn make_azure_store(
        account_name: Option<&str>,
        account_key: Option<&str>,
        container: Option<&str>,
    ) -> StoreConfig {
        StoreConfig {
            backend: StoreBackend::Azure,
            root: None,
            s3: None,
            azure: Some(types::AzureStoreConfig {
                account_name: account_name.unwrap_or("").to_string(),
                account_key: account_key.unwrap_or("").to_string(),
                container: container.unwrap_or("").to_string(),
            }),
            gcs: None,
        }
    }

    fn make_gcs_store(
        project_id: Option<&str>,
        credentials: Option<&str>,
        bucket: Option<&str>,
    ) -> StoreConfig {
        StoreConfig {
            backend: StoreBackend::Gcs,
            root: None,
            s3: None,
            azure: None,
            gcs: Some(types::GcsStoreConfig {
                project_id: project_id.unwrap_or("").to_string(),
                credentials: credentials.unwrap_or("").to_string(),
                bucket: bucket.unwrap_or("").to_string(),
            }),
        }
    }

    // Helper function to construct endpoint configs with all action variants.
    #[allow(clippy::too_many_arguments)] // make_endpoint is a test helper that constructs complete EndpointConfigs; refactoring into multiple helpers would add unnecessary boilerplate in test code
    fn make_endpoint(
        path: &str,
        method: types::HttpMethod,
        action: types::EndpointAction,
        crud: Option<types::CrudConfig>,
        proxy: Option<types::ProxyConfig>,
        static_files: Option<types::StaticFilesConfig>,
        spa_host: Option<types::SpaHostConfig>,
        media: Option<types::MediaConfig>,
        file_store: Option<types::FileStoreConfig>,
        custom_response: Option<types::CustomResponseConfig>,
    ) -> types::EndpointConfig {
        types::EndpointConfig {
            path: path.to_string(),
            methods: vec![method],
            action,
            crud,
            proxy,
            static_files,
            spa_host,
            media,
            file_store,
            custom_response,
            auth: "none".to_string(),
            roles: RolesConfig::Flat(Vec::new()),
            cors: None,
            rate_limit: None,
        }
    }

    #[test]
    fn test_validate_stores_native_no_root() {
        let mut stores = HashMap::new();
        stores.insert("my_native".to_string(), make_native_store(None));
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("root directory must not be empty"));
    }

    #[test]
    fn test_validate_stores_native_with_root() {
        let mut stores = HashMap::new();
        stores.insert(
            "my_native".to_string(),
            make_native_store(Some("/tmp/assets")),
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_stores_native_empty_root() {
        let mut stores = HashMap::new();
        stores.insert("my_native".to_string(), make_native_store(Some("")));
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("root directory must not be empty"));
    }

    #[test]
    fn test_validate_stores_s3_missing_all_fields() {
        let mut stores = HashMap::new();
        stores.insert("my_s3".to_string(), make_s3_store(None, None, None, None));
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 4);
        assert!(errors.iter().any(|e| e.contains("s3.region")));
        assert!(errors.iter().any(|e| e.contains("s3.bucket")));
        assert!(errors.iter().any(|e| e.contains("s3.access_key")));
        assert!(errors.iter().any(|e| e.contains("s3.secret_key")));
    }

    #[test]
    fn test_validate_stores_s3_partial_fields() {
        let mut stores = HashMap::new();
        stores.insert(
            "my_s3".to_string(),
            make_s3_store(Some("us-east-1"), None, None, None),
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 3);
    }

    #[test]
    fn test_validate_stores_s3_complete() {
        let mut stores = HashMap::new();
        stores.insert(
            "my_s3".to_string(),
            make_s3_store(
                Some("us-east-1"),
                Some("my-bucket"),
                Some("key"),
                Some("secret"),
            ),
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_stores_s3_with_optional_fields() {
        let mut stores = HashMap::new();
        stores.insert(
            "my_s3".to_string(),
            types::StoreConfig {
                backend: StoreBackend::S3,
                root: None,
                s3: Some(types::S3StoreConfig {
                    region: "us-east-1".to_string(),
                    bucket: "my-bucket".to_string(),
                    access_key: "key".to_string(),
                    secret_key: "secret".to_string(),
                    endpoint: Some("http://localhost:9000".to_string()),
                    force_path_style: true,
                }),
                azure: None,
                gcs: None,
            },
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_stores_azure_missing_all_fields() {
        let mut stores = HashMap::new();
        stores.insert("my_azure".to_string(), make_azure_store(None, None, None));
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 3);
        assert!(errors.iter().any(|e| e.contains("azure.account_name")));
        assert!(errors.iter().any(|e| e.contains("azure.account_key")));
        assert!(errors.iter().any(|e| e.contains("azure.container")));
    }

    #[test]
    fn test_validate_stores_azure_complete() {
        let mut stores = HashMap::new();
        stores.insert(
            "my_azure".to_string(),
            make_azure_store(Some("myaccount"), Some("mykey"), Some("mycontainer")),
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_stores_gcs_missing_all_fields() {
        let mut stores = HashMap::new();
        stores.insert("my_gcs".to_string(), make_gcs_store(None, None, None));
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 3);
        assert!(errors.iter().any(|e| e.contains("gcs.project_id")));
        assert!(errors.iter().any(|e| e.contains("gcs.credentials")));
        assert!(errors.iter().any(|e| e.contains("gcs.bucket")));
    }

    #[test]
    fn test_validate_stores_gcs_complete() {
        let mut stores = HashMap::new();
        stores.insert(
            "my_gcs".to_string(),
            make_gcs_store(
                Some("my-project"),
                Some("credentials.json"),
                Some("my-bucket"),
            ),
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_stores_conflicting_backend_configs() {
        let mut stores = HashMap::new();
        stores.insert(
            "conflict".to_string(),
            types::StoreConfig {
                backend: StoreBackend::Native,
                root: Some("/tmp".to_string()),
                s3: Some(types::S3StoreConfig {
                    region: "us-east-1".to_string(),
                    bucket: "bucket".to_string(),
                    access_key: "key".to_string(),
                    secret_key: "secret".to_string(),
                    endpoint: None,
                    force_path_style: false,
                }),
                azure: None,
                gcs: None,
            },
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("conflicting backend config sections"));
    }

    #[test]
    fn test_validate_stores_memory_with_root_errors() {
        let mut stores = HashMap::new();
        stores.insert(
            "mem".to_string(),
            types::StoreConfig {
                backend: StoreBackend::Memory,
                root: Some("/tmp".to_string()),
                s3: None,
                azure: None,
                gcs: None,
            },
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("root must not be set for memory backend"));
    }

    #[test]
    fn test_validate_stores_memory_without_root_ok() {
        let mut stores = HashMap::new();
        stores.insert(
            "mem".to_string(),
            types::StoreConfig {
                backend: StoreBackend::Memory,
                root: None,
                s3: None,
                azure: None,
                gcs: None,
            },
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_stores_multiple_stores() {
        let mut stores = HashMap::new();
        stores.insert(
            "native1".to_string(),
            make_native_store(Some("/tmp/assets1")),
        );
        stores.insert(
            "native2".to_string(),
            make_native_store(Some("/tmp/assets2")),
        );
        stores.insert(
            "s3".to_string(),
            make_s3_store(
                Some("us-west-2"),
                Some("cdn-bucket"),
                Some("ak"),
                Some("sk"),
            ),
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_stores_mixed_errors() {
        let mut stores = HashMap::new();
        stores.insert("native_bad".to_string(), make_native_store(None));
        stores.insert("s3_bad".to_string(), make_s3_store(None, None, None, None));
        stores.insert(
            "native_good".to_string(),
            make_native_store(Some("/tmp/good")),
        );
        let mut errors = Vec::new();
        validate_stores(&stores, &mut errors);
        assert_eq!(errors.len(), 5); // 1 native + 4 s3
    }

    #[test]
    fn test_validate_config_staticfiles_missing_store_ref() {
        let mut config = types::AppConfig::default();
        config.endpoints.push(make_endpoint(
            "/static",
            types::HttpMethod::Get,
            types::EndpointAction::StaticFiles,
            None,
            None,
            Some(types::StaticFilesConfig {
                storage: "nonexistent_store".to_string(),
                ..Default::default()
            }),
            None,
            None,
            None,
            None,
        ));
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("nonexistent_store"));
    }

    #[test]
    fn test_validate_config_staticfiles_valid_store_ref() {
        let mut config = types::AppConfig::default();
        config
            .stores
            .insert("assets".to_string(), make_native_store(Some("/tmp/assets")));
        config.endpoints.push(make_endpoint(
            "/static",
            types::HttpMethod::Get,
            types::EndpointAction::StaticFiles,
            None,
            None,
            Some(types::StaticFilesConfig {
                storage: "assets".to_string(),
                ..Default::default()
            }),
            None,
            None,
            None,
            None,
        ));
        let result = validate_config(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_config_spa_host_missing_store_ref() {
        let mut config = types::AppConfig::default();
        config.endpoints.push(make_endpoint(
            "/app",
            types::HttpMethod::Get,
            types::EndpointAction::SpaHost,
            None,
            None,
            None,
            Some(types::SpaHostConfig {
                storage: "missing".to_string(),
                ..Default::default()
            }),
            None,
            None,
            None,
        ));
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("missing"));
    }

    #[test]
    fn test_validate_config_media_missing_refs() {
        let mut config = types::AppConfig::default();
        config.endpoints.push(make_endpoint(
            "/media",
            types::HttpMethod::Get,
            types::EndpointAction::Media,
            None,
            None,
            None,
            None,
            Some(types::MediaConfig {
                storage: "nonexistent".to_string(),
                table: "nonexistent_table".to_string(),
                database: "nonexistent_db".to_string(),
                ..Default::default()
            }),
            None,
            None,
        ));
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("nonexistent"));
    }

    #[test]
    fn test_validate_config_filestore_missing_refs() {
        let mut config = types::AppConfig::default();
        config.endpoints.push(make_endpoint(
            "/files",
            types::HttpMethod::Get,
            types::EndpointAction::FileStore,
            None,
            None,
            None,
            None,
            None,
            Some(types::FileStoreConfig {
                storage: "nonexistent".to_string(),
                table: "nonexistent_table".to_string(),
                database: "nonexistent_db".to_string(),
                ..Default::default()
            }),
            None,
        ));
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("nonexistent"));
    }

    #[test]
    fn test_validate_jwt_revocation_in_memory() {
        let mut config = types::AppConfig::default();
        config.auth.jwt = Some(types::JwtConfig {
            secret: "test-secret".to_string(),
            algorithm: types::JwtAlgorithm::HS256,
            issuer: "test".to_string(),
            audience: String::new(),
            expiry: 3600,
            role_claim: "role".to_string(),
            revocation: Some(types::JwtRevocationConfig {
                store: types::RevocationStoreType::InMemory,
                db_table: None,
                cleanup_interval_secs: Some(3600),
            }),
        });
        let result = validate_config(&config);
        assert!(result.is_ok(), "in_memory revocation should be valid");
    }

    #[test]
    fn test_validate_jwt_revocation_database_no_table() {
        let mut config = types::AppConfig::default();
        config.auth.jwt = Some(types::JwtConfig {
            secret: "test-secret".to_string(),
            algorithm: types::JwtAlgorithm::HS256,
            issuer: "test".to_string(),
            audience: String::new(),
            expiry: 3600,
            role_claim: "role".to_string(),
            revocation: Some(types::JwtRevocationConfig {
                store: types::RevocationStoreType::Database,
                db_table: None,
                cleanup_interval_secs: Some(3600),
            }),
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("db_table must not be empty"),
            "Expected db_table error, got: {msg}"
        );
    }

    #[test]
    fn test_validate_jwt_revocation_database_zero_interval() {
        let mut config = types::AppConfig::default();
        config.auth.jwt = Some(types::JwtConfig {
            secret: "test-secret".to_string(),
            algorithm: types::JwtAlgorithm::HS256,
            issuer: "test".to_string(),
            audience: String::new(),
            expiry: 3600,
            role_claim: "role".to_string(),
            revocation: Some(types::JwtRevocationConfig {
                store: types::RevocationStoreType::Database,
                db_table: Some("token_blacklist".to_string()),
                cleanup_interval_secs: Some(0),
            }),
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("cleanup_interval_secs must be > 0"),
            "Expected cleanup_interval error, got: {msg}"
        );
    }

    #[test]
    fn test_validate_role_mapping_empty_map() {
        let mut config = types::AppConfig::default();
        config.auth.oauth2 = Some(types::OAuth2Config {
            provider: "test".to_string(),
            authorization_url: "https://idp.example.com/authorize".to_string(),
            token_url: "https://idp.example.com/token".to_string(),
            userinfo_url: "https://idp.example.com/userinfo".to_string(),
            client_id: "test-client".to_string(),
            client_secret: "test-secret".to_string(),
            scopes: vec!["openid".to_string()],
            redirect_url: "http://localhost:8080/callback".to_string(),
            success_url: "/".to_string(),
            cookie_name: "token".to_string(),
            state_ttl: 300,
            max_pending_states: 1000,
            role_mapping: Some(types::RoleMappingConfig {
                default_role: "user".to_string(),
                role_claim: "groups".to_string(),
                role_map: std::collections::HashMap::new(),
                match_mode: types::RoleMatchMode::Exact,
            }),
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("role_map must not be empty"),
            "Expected role_map error, got: {msg}"
        );
    }

    #[test]
    fn test_validate_role_mapping_empty_default_role() {
        let mut config = types::AppConfig::default();
        config.auth.oauth2 = Some(types::OAuth2Config {
            provider: "test".to_string(),
            authorization_url: "https://idp.example.com/authorize".to_string(),
            token_url: "https://idp.example.com/token".to_string(),
            userinfo_url: "https://idp.example.com/userinfo".to_string(),
            client_id: "test-client".to_string(),
            client_secret: "test-secret".to_string(),
            scopes: vec!["openid".to_string()],
            redirect_url: "http://localhost:8080/callback".to_string(),
            success_url: "/".to_string(),
            cookie_name: "token".to_string(),
            state_ttl: 300,
            max_pending_states: 1000,
            role_mapping: Some(types::RoleMappingConfig {
                default_role: String::new(),
                role_claim: "groups".to_string(),
                role_map: {
                    let mut map = std::collections::HashMap::new();
                    map.insert("admin-group".to_string(), "admin".to_string());
                    map
                },
                match_mode: types::RoleMatchMode::Exact,
            }),
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("default_role must not be empty"),
            "Expected default_role error, got: {msg}"
        );
    }

    #[test]
    fn test_validate_register_disabled() {
        let mut config = types::AppConfig::default();
        config.auth.register = Some(types::RegisterConfig {
            enabled: false,
            table: String::new(),
            database: String::new(),
            default_role: String::new(),
            password_hash: types::PasswordHashAlgorithm::Argon2id,
        });
        let result = validate_config(&config);
        assert!(
            result.is_ok(),
            "disabled registration should be valid regardless of other fields"
        );
    }

    #[test]
    fn test_validate_register_enabled_no_table() {
        let mut config = types::AppConfig::default();
        config.auth.register = Some(types::RegisterConfig {
            enabled: true,
            table: String::new(),
            database: "main".to_string(),
            default_role: "user".to_string(),
            password_hash: types::PasswordHashAlgorithm::Argon2id,
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("auth.register.table must not be empty"),
            "Expected table error, got: {msg}"
        );
    }

    #[test]
    fn test_validate_register_enabled_no_database() {
        let mut config = types::AppConfig::default();
        config.auth.register = Some(types::RegisterConfig {
            enabled: true,
            table: "users".to_string(),
            database: String::new(),
            default_role: "user".to_string(),
            password_hash: types::PasswordHashAlgorithm::Argon2id,
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("auth.register.database must not be empty"),
            "Expected database error, got: {msg}"
        );
    }

    #[test]
    fn test_validate_register_enabled_no_default_role() {
        let mut config = types::AppConfig::default();
        config.auth.register = Some(types::RegisterConfig {
            enabled: true,
            table: "users".to_string(),
            database: "main".to_string(),
            default_role: String::new(),
            password_hash: types::PasswordHashAlgorithm::Argon2id,
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("auth.register.default_role must not be empty"),
            "Expected default_role error, got: {msg}"
        );
    }

    #[test]
    fn test_validate_register_enabled_valid() {
        let mut config = types::AppConfig::default();
        config.auth.register = Some(types::RegisterConfig {
            enabled: true,
            table: "users".to_string(),
            database: "main".to_string(),
            default_role: "user".to_string(),
            password_hash: types::PasswordHashAlgorithm::Argon2id,
        });
        let result = validate_config(&config);
        assert!(result.is_ok(), "valid registration config should pass");
    }

    #[test]
    fn test_validate_register_enabled_all_errors() {
        let mut config = types::AppConfig::default();
        config.auth.register = Some(types::RegisterConfig {
            enabled: true,
            table: String::new(),
            database: String::new(),
            default_role: String::new(),
            password_hash: types::PasswordHashAlgorithm::Argon2id,
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("auth.register.table"));
        assert!(msg.contains("auth.register.database"));
        assert!(msg.contains("auth.register.default_role"));
    }

    #[test]
    fn test_validate_tables_non_jsonb_with_validation_rejected() {
        let mut config = types::AppConfig::default();
        config.databases.insert(
            "main".to_string(),
            types::DatabaseConfig {
                driver: types::DatabaseDriver::Sqlite,
                url: "sqlite://:memory:".to_string(),
                min_connections: 1,
                max_connections: 10,
                auto_migrate: false,
                allow_destructive: false,
                acquire_timeout: 5,
            },
        );
        config.tables.push(types::TableConfig {
            name: "test_table".to_string(),
            database: "main".to_string(),
            columns: vec![
                types::ColumnConfig {
                    name: "id".to_string(),
                    column_type: types::ColumnType::Serial,
                    primary_key: true,
                    nullable: false,
                    default: None,
                    unique: false,
                    indexed: false,
                    validation_schema: None,
                    validation: None,
                    validation_schema_ref: None,
                },
                types::ColumnConfig {
                    name: "metadata".to_string(),
                    column_type: types::ColumnType::Text,
                    primary_key: false,
                    nullable: true,
                    default: None,
                    unique: false,
                    indexed: false,
                    validation_schema: Some("./schema.json".to_string()),
                    validation: None,
                    validation_schema_ref: None,
                },
            ],
            foreign_keys: vec![],
        });
        let result = validate_config(&config);
        assert!(
            result.is_err(),
            "non-JSONB column with validation_schema should be rejected"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("not jsonb or json"),
            "Error should mention non-JSONB type, got: {msg}"
        );
        assert!(
            msg.contains("metadata"),
            "Error should mention column name, got: {msg}"
        );
    }

    #[test]
    fn test_validate_tables_non_jsonb_with_inline_validation_rejected() {
        let mut config = types::AppConfig::default();
        config.databases.insert(
            "main".to_string(),
            types::DatabaseConfig {
                driver: types::DatabaseDriver::Sqlite,
                url: "sqlite://:memory:".to_string(),
                min_connections: 1,
                max_connections: 10,
                auto_migrate: false,
                allow_destructive: false,
                acquire_timeout: 5,
            },
        );
        let inline_schema =
            serde_yaml::from_value(serde_yaml::Value::Mapping(serde_yaml::Mapping::new())).unwrap();
        config.tables.push(types::TableConfig {
            name: "test_table".to_string(),
            database: "main".to_string(),
            columns: vec![
                types::ColumnConfig {
                    name: "id".to_string(),
                    column_type: types::ColumnType::Serial,
                    primary_key: true,
                    nullable: false,
                    default: None,
                    unique: false,
                    indexed: false,
                    validation_schema: None,
                    validation: None,
                    validation_schema_ref: None,
                },
                types::ColumnConfig {
                    name: "data".to_string(),
                    column_type: types::ColumnType::Integer,
                    primary_key: false,
                    nullable: true,
                    default: None,
                    unique: false,
                    indexed: false,
                    validation_schema: None,
                    validation: Some(inline_schema),
                    validation_schema_ref: None,
                },
            ],
            foreign_keys: vec![],
        });
        let result = validate_config(&config);
        assert!(
            result.is_err(),
            "non-JSONB column with inline validation should be rejected"
        );
    }

    #[test]
    fn test_validate_tables_multiple_validation_fields_rejected() {
        let mut config = types::AppConfig::default();
        config.databases.insert(
            "main".to_string(),
            types::DatabaseConfig {
                driver: types::DatabaseDriver::Sqlite,
                url: "sqlite://:memory:".to_string(),
                min_connections: 1,
                max_connections: 10,
                auto_migrate: false,
                allow_destructive: false,
                acquire_timeout: 5,
            },
        );
        let inline_schema =
            serde_yaml::from_value(serde_yaml::Value::Mapping(serde_yaml::Mapping::new())).unwrap();
        config.tables.push(types::TableConfig {
            name: "test_table".to_string(),
            database: "main".to_string(),
            columns: vec![
                types::ColumnConfig {
                    name: "id".to_string(),
                    column_type: types::ColumnType::Serial,
                    primary_key: true,
                    nullable: false,
                    default: None,
                    unique: false,
                    indexed: false,
                    validation_schema: None,
                    validation: None,
                    validation_schema_ref: None,
                },
                types::ColumnConfig {
                    name: "data".to_string(),
                    column_type: types::ColumnType::Jsonb,
                    primary_key: false,
                    nullable: true,
                    default: None,
                    unique: false,
                    indexed: false,
                    validation_schema: Some("./schema.json".to_string()),
                    validation: Some(inline_schema),
                    validation_schema_ref: None,
                },
            ],
            foreign_keys: vec![],
        });
        let result = validate_config(&config);
        assert!(
            result.is_err(),
            "column with multiple validation fields should be rejected"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("multiple schema validation fields") || msg.contains("at most one"),
            "Error should mention mutual exclusivity, got: {msg}"
        );
    }

    #[test]
    fn test_validate_tables_jsonb_with_single_validation_valid() {
        let mut config = types::AppConfig::default();
        config.databases.insert(
            "main".to_string(),
            types::DatabaseConfig {
                driver: types::DatabaseDriver::Sqlite,
                url: "sqlite://:memory:".to_string(),
                min_connections: 1,
                max_connections: 10,
                auto_migrate: false,
                allow_destructive: false,
                acquire_timeout: 5,
            },
        );
        let inline_schema =
            serde_yaml::from_value(serde_yaml::Value::Mapping(serde_yaml::Mapping::new())).unwrap();
        config.tables.push(types::TableConfig {
            name: "test_table".to_string(),
            database: "main".to_string(),
            columns: vec![
                types::ColumnConfig {
                    name: "id".to_string(),
                    column_type: types::ColumnType::Serial,
                    primary_key: true,
                    nullable: false,
                    default: None,
                    unique: false,
                    indexed: false,
                    validation_schema: None,
                    validation: None,
                    validation_schema_ref: None,
                },
                types::ColumnConfig {
                    name: "data".to_string(),
                    column_type: types::ColumnType::Jsonb,
                    primary_key: false,
                    nullable: true,
                    default: None,
                    unique: false,
                    indexed: false,
                    validation_schema: None,
                    validation: Some(inline_schema),
                    validation_schema_ref: None,
                },
            ],
            foreign_keys: vec![],
        });
        let result = validate_config(&config);
        assert!(
            result.is_ok(),
            "single validation field on JSONB column should be valid"
        );
    }
}

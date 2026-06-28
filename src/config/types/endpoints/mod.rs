//! Endpoint definitions and action-specific configurations.
//!
//! Types are organized into submodules by action type:
//! - `common` - EndpointConfig, HttpMethod, RolesConfig, EndpointAction, and shared helpers
//! - `crud` - CrudConfig, PaginationConfig, FilteringConfig, SortingConfig
//! - `proxy` - ProxyConfig, PathRewriteConfig, ProxyTimeouts
//! - `static_files` - StaticFilesConfig, UploadConfig, ImageResizeConfig, StreamingConfig
//! - `media` - MediaConfig and all media-specific config types
//! - `file_store` - FileStoreConfig and file store-specific config types
//! - `spa_host` - SpaHostConfig
//! - `custom_response` - CustomResponseConfig
pub mod common;
pub mod crud;
pub mod custom_response;
pub mod file_store;
pub mod media;
pub mod proxy;
pub mod spa_host;
pub mod static_files;

pub use common::*;
pub use crud::*;
pub use custom_response::*;
pub use file_store::*;
pub use media::*;
pub use proxy::*;
pub use spa_host::*;
pub use static_files::*;

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[test]
    fn test_deserialize_flat_roles() {
        let yaml = r#"roles: ["admin", "editor"]"#;
        #[derive(Deserialize)]
        struct Wrapper {
            roles: RolesConfig,
        }
        let wrapper: Wrapper = serde_yaml::from_str(yaml).expect("should deserialize flat roles");
        match &wrapper.roles {
            RolesConfig::Flat(roles) => {
                assert_eq!(roles, &["admin", "editor"]);
            }
            _ => panic!("expected Flat variant"),
        }
    }

    #[test]
    fn test_deserialize_method_specific_roles() {
        let yaml = r#"
roles:
  get: ["admin", "editor"]
  post: ["admin"]
  put: ["admin"]
  patch: ["admin"]
  delete: ["admin"]
  head: ["admin", "editor"]
  options: []
"#;
        #[derive(Deserialize)]
        struct Wrapper {
            roles: RolesConfig,
        }
        let wrapper: Wrapper =
            serde_yaml::from_str(yaml).expect("should deserialize method-specific roles");
        match &wrapper.roles {
            RolesConfig::MethodSpecific(ms) => {
                assert_eq!(ms.get.as_ref().unwrap(), &["admin", "editor"]);
                assert_eq!(ms.post.as_ref().unwrap(), &["admin"]);
                assert_eq!(ms.put.as_ref().unwrap(), &["admin"]);
                assert_eq!(ms.patch.as_ref().unwrap(), &["admin"]);
                assert_eq!(ms.delete.as_ref().unwrap(), &["admin"]);
                assert_eq!(ms.head.as_ref().unwrap(), &["admin", "editor"]);
                assert!(ms.options.as_ref().unwrap().is_empty());
            }
            _ => panic!("expected MethodSpecific variant"),
        }
    }

    #[test]
    fn test_deserialize_partial_method_specific_roles() {
        let yaml = r#"
roles:
  get: ["admin"]
  delete: ["admin", "editor"]
"#;
        #[derive(Deserialize)]
        struct Wrapper {
            roles: RolesConfig,
        }
        let wrapper: Wrapper =
            serde_yaml::from_str(yaml).expect("should deserialize partial roles");
        match &wrapper.roles {
            RolesConfig::MethodSpecific(ms) => {
                assert_eq!(ms.get.as_ref().unwrap(), &["admin"]);
                assert!(ms.post.is_none());
                assert!(ms.put.is_none());
                assert!(ms.patch.is_none());
                assert_eq!(ms.delete.as_ref().unwrap(), &["admin", "editor"]);
                assert!(ms.head.is_none());
                assert!(ms.options.is_none());
            }
            _ => panic!("expected MethodSpecific variant"),
        }
    }

    #[test]
    fn test_deserialize_empty_roles() {
        let yaml = r#"roles: []"#;
        #[derive(Deserialize)]
        struct Wrapper {
            roles: RolesConfig,
        }
        let wrapper: Wrapper = serde_yaml::from_str(yaml).expect("should deserialize empty roles");
        match &wrapper.roles {
            RolesConfig::Flat(roles) => assert!(roles.is_empty()),
            _ => panic!("expected Flat variant"),
        }
    }

    #[test]
    fn test_deserialize_default_roles() {
        let yaml = r#""#;
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(default)]
            roles: RolesConfig,
        }
        let wrapper: Wrapper =
            serde_yaml::from_str(yaml).expect("should deserialize default roles");
        match &wrapper.roles {
            RolesConfig::Flat(roles) => assert!(roles.is_empty()),
            _ => panic!("expected Flat variant"),
        }
    }

    #[test]
    fn test_clone_for_method_flat_all_methods() {
        let config = RolesConfig::Flat(vec!["admin".to_string(), "user".to_string()]);
        assert_eq!(
            config.clone_for_method(HttpMethod::Get),
            vec!["admin", "user"]
        );
        assert_eq!(
            config.clone_for_method(HttpMethod::Post),
            vec!["admin", "user"]
        );
        assert_eq!(
            config.clone_for_method(HttpMethod::Put),
            vec!["admin", "user"]
        );
        assert_eq!(
            config.clone_for_method(HttpMethod::Patch),
            vec!["admin", "user"]
        );
        assert_eq!(
            config.clone_for_method(HttpMethod::Delete),
            vec!["admin", "user"]
        );
        assert_eq!(
            config.clone_for_method(HttpMethod::Head),
            vec!["admin", "user"]
        );
        assert_eq!(
            config.clone_for_method(HttpMethod::Options),
            vec!["admin", "user"]
        );
    }

    #[test]
    fn test_clone_for_method_specific_returns_method_roles() {
        let ms = MethodSpecificRoles {
            get: Some(vec!["admin".to_string()]),
            post: Some(vec!["editor".to_string(), "admin".to_string()]),
            put: None,
            patch: None,
            delete: Some(vec!["admin".to_string()]),
            head: None,
            options: Some(vec![]),
        };
        let config = RolesConfig::MethodSpecific(ms);
        assert_eq!(config.clone_for_method(HttpMethod::Get), vec!["admin"]);
        assert_eq!(
            config.clone_for_method(HttpMethod::Post),
            vec!["editor", "admin"]
        );
        assert!(config.clone_for_method(HttpMethod::Put).is_empty());
        assert!(config.clone_for_method(HttpMethod::Patch).is_empty());
        assert_eq!(config.clone_for_method(HttpMethod::Delete), vec!["admin"]);
        assert!(config.clone_for_method(HttpMethod::Head).is_empty());
        assert!(config.clone_for_method(HttpMethod::Options).is_empty());
    }

    #[test]
    fn test_clone_for_method_flat_empty_allows_all() {
        let config = RolesConfig::Flat(vec![]);
        assert!(config.clone_for_method(HttpMethod::Get).is_empty());
        assert!(config.clone_for_method(HttpMethod::Post).is_empty());
    }

    #[test]
    fn test_default_roles_config_is_empty_flat() {
        let config = RolesConfig::default();
        assert!(matches!(config, RolesConfig::Flat(ref v) if v.is_empty()));
    }

    #[test]
    fn test_default_method_specific_has_all_none() {
        let ms = MethodSpecificRoles::default();
        assert!(ms.get.is_none());
        assert!(ms.post.is_none());
        assert!(ms.put.is_none());
        assert!(ms.patch.is_none());
        assert!(ms.delete.is_none());
        assert!(ms.head.is_none());
        assert!(ms.options.is_none());
    }

    /// Test that an endpoint with flat roles deserializes correctly.
    #[test]
    fn test_endpoint_config_flat_roles_roundtrip() {
        let yaml = r#"
path: "/api/test"
methods: ["get", "post"]
action: "crud"
auth: "jwt"
roles: ["admin", "editor"]
"#;
        #[derive(Deserialize)]
        #[allow(dead_code)] // Wrapper is a test-only struct for deserialization round-trip verification
        struct Wrapper {
            path: String,
            methods: Vec<HttpMethod>,
            action: EndpointAction,
            #[serde(default)]
            crud: Option<CrudConfig>,
            #[serde(default = "super::common::default_auth_none")]
            auth: String,
            #[serde(default)]
            roles: RolesConfig,
        }
        let wrapper: Wrapper = serde_yaml::from_str(yaml).expect("should deserialize endpoint");
        match &wrapper.roles {
            RolesConfig::Flat(roles) => {
                assert_eq!(roles, &["admin", "editor"]);
            }
            _ => panic!("expected Flat variant"),
        }
        assert_eq!(wrapper.auth, "jwt");
        assert_eq!(wrapper.methods.len(), 2);
    }

    /// Test that an endpoint with method-specific roles deserializes correctly.
    #[test]
    fn test_endpoint_config_method_specific_roles_roundtrip() {
        let yaml = r#"
path: "/api/test"
methods: ["get", "post", "delete"]
action: "crud"
auth: "jwt"
roles:
  get: ["admin", "editor"]
  post: ["admin"]
  delete: ["admin"]
"#;
        #[derive(Deserialize)]
        #[allow(dead_code)] // Wrapper is a test-only struct for deserialization round-trip verification
        struct Wrapper {
            path: String,
            methods: Vec<HttpMethod>,
            action: EndpointAction,
            #[serde(default)]
            crud: Option<CrudConfig>,
            #[serde(default = "super::common::default_auth_none")]
            auth: String,
            #[serde(default)]
            roles: RolesConfig,
        }
        let wrapper: Wrapper = serde_yaml::from_str(yaml).expect("should deserialize endpoint");
        match &wrapper.roles {
            RolesConfig::MethodSpecific(ms) => {
                assert_eq!(ms.get.as_ref().unwrap(), &["admin", "editor"]);
                assert_eq!(ms.post.as_ref().unwrap(), &["admin"]);
                assert_eq!(ms.delete.as_ref().unwrap(), &["admin"]);
                assert!(ms.patch.is_none());
            }
            _ => panic!("expected MethodSpecific variant"),
        }
    }
}

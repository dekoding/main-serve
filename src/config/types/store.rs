/// Named storage backend configuration types.
///
/// Defines `StoreConfig` and backend-specific configs (S3, Azure, GCS)
/// that endpoints reference by name via the `stores` section of the YAML config.
use serde::Deserialize;

/// Top-level store definition - references a named storage backend.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// StoreConfig
pub struct StoreConfig {
    /// Storage backend type: "native" (local disk), "s3" (Amazon S3),
    /// "azure" (Azure Blob Storage), "gcs" (Google Cloud Storage),
    /// "memory" (in-memory for testing).
    pub backend: StoreBackend,
    /// Root directory for local (native) storage. Required when backend is "native".
    #[serde(default)]
    pub root: Option<String>,
    /// S3 configuration. Required when backend is "s3".
    #[serde(default)]
    pub s3: Option<S3StoreConfig>,
    /// Azure Blob Storage configuration. Required when backend is "azure".
    #[serde(default)]
    pub azure: Option<AzureStoreConfig>,
    /// Google Cloud Storage configuration. Required when backend is "gcs".
    #[serde(default)]
    pub gcs: Option<GcsStoreConfig>,
}

/// Storage backend type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
/// StoreBackend
pub enum StoreBackend {
    #[default]
    Native,
    S3,
    Azure,
    Gcs,
    /// In-memory storage backend for testing.
    /// No configuration required - no backend-specific config section needed.
    Memory,
}

/// S3 storage backend configuration.
#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// S3StoreConfig
pub struct S3StoreConfig {
    /// AWS region (e.g. "us-east-1").
    pub region: String,
    /// S3 bucket name.
    pub bucket: String,
    /// AWS access key ID.
    pub access_key: String,
    /// AWS secret access key.
    #[serde(skip_serializing, default)]
    pub secret_key: String,
    /// Optional custom endpoint URL for S3-compatible services.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Use path-style URLs (true for `MinIO`, `LocalStack`, Ceph).
    #[serde(default)]
    pub force_path_style: bool,
}

impl std::fmt::Debug for S3StoreConfig {
    /// Formats the S3 config for debug output, redacting the secret key value.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3StoreConfig")
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field("access_key", &self.access_key)
            .field("secret_key", &"[REDACTED]")
            .field("endpoint", &self.endpoint)
            .field("force_path_style", &self.force_path_style)
            .finish()
    }
}

/// Azure Blob Storage backend configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// AzureStoreConfig
pub struct AzureStoreConfig {
    /// Azure storage account name.
    pub account_name: String,
    /// Azure storage account key.
    #[serde(skip_serializing, default)]
    pub account_key: String,
    /// Azure container name.
    pub container: String,
}

/// Google Cloud Storage backend configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// GcsStoreConfig
pub struct GcsStoreConfig {
    /// GCP project ID.
    pub project_id: String,
    /// GCP credentials JSON string or path.
    #[serde(skip_serializing, default)]
    pub credentials: String,
    /// GCS bucket name.
    pub bucket: String,
}

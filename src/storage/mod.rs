/// Storage abstraction for file operations.
///
/// Provides a unified, async interface for all file I/O operations,
/// supporting multiple backends (native filesystem, in-memory for testing,
/// S3, Azure Blob Storage, Google Cloud Storage).
pub mod backend;
/// error
pub mod error;
/// metadata
pub mod metadata;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::types::StoreConfig;
pub use error::StorageError;
pub use metadata::{DirEntry, FileMetadata};

/// Result type for storage operations.
pub type Result<T> = std::result::Result<T, StorageError>;

/// Unified storage interface for all file operations.
///
/// This trait abstracts file I/O operations to support multiple backends.
/// The native filesystem implementation is provided by `NativeStorage`.
#[async_trait::async_trait]
/// Storage
pub trait Storage: Send + Sync {
    /// Check if a file or directory exists at the given path.
    async fn exists(&self, path: &Path) -> bool;

    /// Check if the path points to a file.
    async fn is_file(&self, path: &Path) -> bool;

    /// Check if the path points to a directory.
    async fn is_dir(&self, path: &Path) -> bool;

    /// Read the entire contents of a file into bytes.
    async fn read(&self, path: &Path) -> Result<Vec<u8>>;

    /// Open a file and return a streaming reader.
    async fn open(&self, path: &Path) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>>;

    /// Open a file and seek to the given offset, returning a streaming reader.
    ///
    /// This allows efficient range requests and streaming without reading the
    /// entire file into memory first. For native filesystem backends, this
    /// opens the file and seeks to the offset. For cloud backends (S3, Azure,
    /// GCS), this uses native HTTP range request support.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::NotFound` if the file does not exist.
    /// Returns `StorageError::Internal` if the file cannot be opened or the
    /// offset cannot be applied.
    async fn seek_read(
        &self,
        path: &Path,
        offset: u64,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>>;

    /// Write bytes to a file, creating it if it doesn't exist.
    async fn write(&self, path: &Path, contents: &[u8]) -> Result<()>;

    /// Append bytes to a file, creating it if it doesn't exist.
    async fn append(&self, path: &Path, contents: &[u8]) -> Result<()>;

    /// Delete a file.
    async fn delete(&self, path: &Path) -> Result<()>;

    /// Get metadata for a file or directory.
    async fn metadata(&self, path: &Path) -> Result<FileMetadata>;

    /// List entries in a directory.
    async fn list(&self, dir: &Path) -> Result<Vec<DirEntry>>;

    /// Create a directory, returning an error if parent directories don't exist.
    async fn create_dir(&self, path: &Path) -> Result<()>;

    /// Create a directory and all its parents, if they don't exist.
    async fn create_dir_all(&self, path: &Path) -> Result<()>;

    /// Remove an empty directory.
    async fn remove_dir(&self, path: &Path) -> Result<()>;

    /// Remove a directory and all its contents recursively.
    async fn remove_dir_all(&self, path: &Path) -> Result<()>;

    /// Rename or move a file or directory.
    async fn rename(&self, from: &Path, to: &Path) -> Result<()>;

    /// Copy a file to a new location.
    async fn copy(&self, from: &Path, to: &Path) -> Result<()>;

    /// Canonicalize a path, resolving all symbolic links and relative components.
    async fn canonicalize(&self, path: &Path) -> Result<PathBuf>;

    /// Get the size of a file in bytes.
    async fn size(&self, path: &Path) -> Result<u64>;

    /// Return the root path for this storage backend.
    ///
    /// For native filesystem stores, returns `Some(root_path)` pointing
    /// to the directory that serves as the store root. For cloud stores
    /// (S3, Azure, GCS), returns `None` since the root is conceptual
    /// (the bucket/container itself).
    fn root_path(&self) -> Option<PathBuf> {
        None
    }
}

/// Create a storage backend instance from a store configuration.
///
/// Supports:
/// - `native`: Real filesystem (requires `root` path in config)
/// - `memory`: In-memory backend for testing (no additional config needed)
/// - `s3`: AWS S3
/// - `azure`: Azure Blob Storage
/// - `gcs`: Google Cloud Storage
///
/// # Errors
///
/// Returns `StorageError::InvalidBackend` if the required backend-specific config
/// is missing (e.g. no `root` for native, no `azure` section for azure).
/// Returns `StorageError::Internal` if a cloud feature is not compiled in (S3, Azure, GCS).
pub async fn create_store(config: &StoreConfig) -> Result<Arc<dyn Storage>> {
    match config.backend {
        crate::config::types::StoreBackend::Memory => {
            Ok(Arc::new(backend::memory::MemoryStorage::new()) as Arc<dyn Storage>)
        }
        crate::config::types::StoreBackend::Native => {
            let root = config
                .root
                .as_ref()
                .ok_or_else(|| {
                    StorageError::InvalidBackend("native backend requires 'root' field".to_string())
                })?
                .clone();
            let path = PathBuf::from(root);
            Ok(Arc::new(backend::native::NativeStorage::new(path)))
        }
        crate::config::types::StoreBackend::S3 => {
            #[cfg(feature = "s3")]
            {
                let storage = backend::s3::S3Storage::new(config).await?;
                Ok(Arc::new(storage) as Arc<dyn Storage>)
            }
            #[cfg(not(feature = "s3"))]
            {
                Err(StorageError::Internal(
                    "S3 support is not compiled in - enable the 's3' feature flag".to_string(),
                ))
            }
        }
        crate::config::types::StoreBackend::Azure => {
            #[cfg(feature = "azure")]
            {
                let storage = backend::azure::AzureStorage::new(config)?;
                Ok(Arc::new(storage) as Arc<dyn Storage>)
            }
            #[cfg(not(feature = "azure"))]
            {
                Err(StorageError::Internal(
                    "Azure support is not compiled in - enable the 'azure' feature flag"
                        .to_string(),
                ))
            }
        }
        crate::config::types::StoreBackend::Gcs => {
            #[cfg(feature = "gcs")]
            {
                let storage = backend::gcs::GcsStorage::new(config)?;
                Ok(Arc::new(storage) as Arc<dyn Storage>)
            }
            #[cfg(not(feature = "gcs"))]
            {
                Err(StorageError::Internal(
                    "GCS support is not compiled in - enable the 'gcs' feature flag".to_string(),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{StoreBackend, StoreConfig};

    fn block_on_future<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("Failed to build tokio runtime")
            .block_on(f)
    }

    #[test]
    fn test_create_store_native() {
        let config = StoreConfig {
            backend: StoreBackend::Native,
            root: Some("/tmp/test_store".to_string()),
            ..Default::default()
        };
        let result = block_on_future(create_store(&config));
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_store_native_missing_root() {
        let config = StoreConfig {
            backend: StoreBackend::Native,
            root: None,
            ..Default::default()
        };
        let result = block_on_future(create_store(&config));
        assert!(result.is_err());
    }

    #[test]
    fn test_create_store_memory() {
        let config = StoreConfig {
            backend: StoreBackend::Memory,
            ..Default::default()
        };
        let result = block_on_future(create_store(&config));
        assert!(result.is_ok());
    }

    #[test]
    #[cfg(not(feature = "s3"))]
    fn test_create_store_s3_disabled() {
        let config = StoreConfig {
            backend: StoreBackend::S3,
            s3: Some(Default::default()),
            ..Default::default()
        };
        let result = block_on_future(create_store(&config));
        assert!(matches!(result, Err(StorageError::Internal(_))));
    }

    #[test]
    #[cfg(not(feature = "azure"))]
    fn test_create_store_azure_disabled() {
        let config = StoreConfig {
            backend: StoreBackend::Azure,
            azure: Some(Default::default()),
            ..Default::default()
        };
        let result = block_on_future(create_store(&config));
        assert!(matches!(result, Err(StorageError::Internal(_))));
    }

    #[test]
    #[cfg(not(feature = "gcs"))]
    fn test_create_store_gcs_disabled() {
        let config = StoreConfig {
            backend: StoreBackend::Gcs,
            gcs: Some(Default::default()),
            ..Default::default()
        };
        let result = block_on_future(create_store(&config));
        assert!(matches!(result, Err(StorageError::Internal(_))));
    }
}

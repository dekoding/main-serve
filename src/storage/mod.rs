/// Storage abstraction for file operations.
///
/// Provides a unified, async interface for all file I/O operations,
/// supporting multiple backends (native filesystem, in-memory for testing, etc.)
pub mod backend;
pub mod error;
pub mod metadata;

use std::path::{Path, PathBuf};

pub use error::StorageError;
pub use metadata::{DirEntry, FileMetadata};

/// Result type for storage operations.
pub type Result<T> = std::result::Result<T, StorageError>;

/// Unified storage interface for all file operations.
///
/// This trait abstracts file I/O operations to support multiple backends.
/// The native filesystem implementation is provided by `NativeStorage`.
#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    /// Check if a file or directory exists at the given path.
    async fn exists(&self, path: &Path) -> bool;

    /// Check if the path points to a file.
    async fn is_file(&self, path: &Path) -> bool;

    /// Check if the path points to a directory.
    async fn is_dir(&self, path: &Path) -> bool;

    /// Read the entire contents of a file into bytes.
    async fn read(&self, path: &Path) -> Result<Vec<u8>>;

    /// Read a file as a streaming reader.
    async fn open(&self, path: &Path) -> Result<tokio::fs::File>;

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
}

/// Get a storage backend instance based on the backend type.
///
/// Currently supports:
/// - `native`: Real filesystem (default)
/// - `memory`: In-memory backend for testing
pub fn get_backend(backend_type: &str) -> Result<Box<dyn Storage>> {
    match backend_type {
        "native" | "" => Ok(Box::new(backend::native::NativeStorage::new())),
        "memory" => Ok(Box::new(backend::memory::MemoryStorage::new())),
        other => Err(StorageError::InvalidBackend(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_backend_native() {
        let result = get_backend("native");
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_backend_default_empty_string() {
        let result = get_backend("");
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_backend_memory() {
        let result = get_backend("memory");
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_backend_invalid() {
        let result = get_backend("invalid_backend");
        assert!(matches!(result, Err(StorageError::InvalidBackend(_))));
    }

    #[test]
    fn test_get_backend_unknown() {
        let result = get_backend("s3");
        assert!(result.is_err());
    }
}

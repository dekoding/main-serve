use std::path::PathBuf;

use thiserror::Error;

/// Error type for storage operations.
#[derive(Debug, Error)]
/// StorageError
pub enum StorageError {
    /// File or directory not found.
    #[error("Not found: {0:?}")]
    NotFound(PathBuf),

    /// File or directory already exists.
    #[error("Already exists: {0:?}")]
    AlreadyExists(PathBuf),

    /// Directory not empty.
    #[error("Directory not empty: {0:?}")]
    DirectoryNotEmpty(PathBuf),

    /// Permission denied.
    #[error("Permission denied: {0}")]
    Forbidden(String),

    /// Invalid filename.
    #[error("Invalid filename: {0}")]
    InvalidFilename(String),

    /// IO operation failed.
    #[error("IO error for {0:?}: {1}")]
    Io(PathBuf, #[source] std::io::Error),

    /// Invalid storage backend specified.
    #[error("Invalid storage backend: {0}")]
    InvalidBackend(String),

    /// Internal error (should not occur in production).
    #[error("Internal error: {0}")]
    Internal(String),

    /// Cloud service is unavailable or returning errors.
    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    /// Cloud authentication/authorization failure.
    #[error("Authentication failed: {0}")]
    Authentication(String),
}

impl From<StorageError> for crate::error::AppError {
    /// Converts a storage-layer error into a generic application error for response handling.
    fn from(err: StorageError) -> Self {
        match err {
            StorageError::NotFound(path) => {
                crate::error::AppError::NotFound(format!("File not found: {}", path.display()))
            }
            StorageError::AlreadyExists(path) => crate::error::AppError::FileOperation(format!(
                "File already exists: {}",
                path.display()
            )),
            StorageError::DirectoryNotEmpty(path) => crate::error::AppError::FileOperation(
                format!("Directory not empty: {}", path.display()),
            ),
            StorageError::Forbidden(msg) => crate::error::AppError::Forbidden(msg.clone()),
            StorageError::InvalidFilename(msg) => {
                crate::error::AppError::BadRequest(format!("Invalid filename: {msg}"))
            }
            StorageError::Io(path, err) => crate::error::AppError::FileOperation(format!(
                "File operation failed: {}: {err}",
                path.display()
            )),
            StorageError::InvalidBackend(msg) => {
                crate::error::AppError::Config(format!("Invalid storage backend: {msg}"))
            }
            StorageError::Internal(msg) => crate::error::AppError::Internal(msg.clone()),
            StorageError::ServiceUnavailable(msg) => {
                crate::error::AppError::ServiceUnavailable(msg.clone())
            }
            StorageError::Authentication(msg) => crate::error::AppError::Auth(msg.clone()),
        }
    }
}

use std::path::{Path, PathBuf};

use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::storage::{DirEntry, FileMetadata, Result, Storage, StorageError};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

/// Native filesystem storage backend.
///
/// Uses tokio::fs for all file operations.
#[derive(Clone, Copy)]
pub struct NativeStorage;

impl NativeStorage {
    /// Create a new native storage instance.
    pub fn new() -> Self {
        Self
    }
}

impl Default for NativeStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Storage for NativeStorage {
    async fn exists(&self, path: &Path) -> bool {
        fs::try_exists(path).await.unwrap_or(false)
    }

    async fn is_file(&self, path: &Path) -> bool {
        fs::metadata(path).await.is_ok_and(|m| m.is_file())
    }

    async fn is_dir(&self, path: &Path) -> bool {
        fs::metadata(path).await.is_ok_and(|m| m.is_dir())
    }

    async fn read(&self, path: &Path) -> Result<Vec<u8>> {
        fs::read(path)
            .await
            .map_err(|e| StorageError::Io(path.to_path_buf(), e))
    }

    async fn open(&self, path: &Path) -> Result<tokio::fs::File> {
        fs::File::open(path)
            .await
            .map_err(|e| StorageError::Io(path.to_path_buf(), e))
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> Result<()> {
        fs::write(path, contents)
            .await
            .map_err(|e| StorageError::Io(path.to_path_buf(), e))
    }

    async fn append(&self, path: &Path, contents: &[u8]) -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .await
            .map_err(|e| StorageError::Io(path.to_path_buf(), e))?;

        file.write_all(contents)
            .await
            .map_err(|e| StorageError::Io(path.to_path_buf(), e))?;

        Ok(())
    }

    async fn delete(&self, path: &Path) -> Result<()> {
        fs::remove_file(path)
            .await
            .map_err(|e| StorageError::Io(path.to_path_buf(), e))
    }

    async fn metadata(&self, path: &Path) -> Result<FileMetadata> {
        let meta = fs::metadata(path)
            .await
            .map_err(|e| StorageError::Io(path.to_path_buf(), e))?;

        let metadata = FileMetadata::new(
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            meta.is_file(),
            meta.len(),
        );

        let created = meta.created().ok();
        let modified = meta.modified().ok();

        Ok(FileMetadata {
            created,
            modified,
            ..metadata
        })
    }

    async fn list(&self, dir: &Path) -> Result<Vec<DirEntry>> {
        let mut entries = Vec::new();
        let mut read_dir = fs::read_dir(dir)
            .await
            .map_err(|e| StorageError::Io(dir.to_path_buf(), e))?;

        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            match entry.metadata().await {
                Ok(meta) => {
                    let is_dir = meta.is_dir();
                    let size = meta.len();
                    #[cfg(unix)]
                    let (mode, uid, gid) = (meta.mode(), meta.uid(), meta.gid());
                    #[cfg(not(unix))]
                    let (mode, uid, gid) = (0, 0, 0);

                    let modified = meta.modified().ok();

                    entries.push(
                        DirEntry::new(name, is_dir, size)
                            .with_path(path)
                            .with_mode(mode)
                            .with_uid(uid)
                            .with_gid(gid)
                            .with_modified(modified),
                    );
                }
                Err(e) => {
                    return Err(StorageError::Io(dir.to_path_buf(), e));
                }
            }
        }

        Ok(entries)
    }

    async fn create_dir(&self, path: &Path) -> Result<()> {
        fs::create_dir(path)
            .await
            .map_err(|e| StorageError::Io(path.to_path_buf(), e))
    }

    async fn create_dir_all(&self, path: &Path) -> Result<()> {
        fs::create_dir_all(path)
            .await
            .map_err(|e| StorageError::Io(path.to_path_buf(), e))
    }

    async fn remove_dir(&self, path: &Path) -> Result<()> {
        fs::remove_dir(path)
            .await
            .map_err(|e| StorageError::Io(path.to_path_buf(), e))
    }

    async fn remove_dir_all(&self, path: &Path) -> Result<()> {
        // Use tokio::fs::remove_dir_all if available, otherwise implement manually
        #[cfg(target_os = "windows")]
        {
            fs::remove_dir_all(path)
                .await
                .map_err(|e| StorageError::Io(path.to_path_buf(), e))
        }
        #[cfg(not(target_os = "windows"))]
        {
            // For non-Windows platforms, we can use the async version
            fs::remove_dir_all(path)
                .await
                .map_err(|e| StorageError::Io(path.to_path_buf(), e))
        }
    }

    async fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        fs::rename(from, to)
            .await
            .map_err(|e| StorageError::Io(from.to_path_buf(), e))
    }

    async fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        fs::copy(from, to)
            .await
            .map_err(|e| StorageError::Io(from.to_path_buf(), e))?;
        Ok(())
    }

    async fn canonicalize(&self, path: &Path) -> Result<PathBuf> {
        fs::canonicalize(path)
            .await
            .map_err(|e| StorageError::Io(path.to_path_buf(), e))
    }

    async fn size(&self, path: &Path) -> Result<u64> {
        let meta = fs::metadata(path)
            .await
            .map_err(|e| StorageError::Io(path.to_path_buf(), e))?;
        Ok(meta.len())
    }
}

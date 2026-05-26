use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::storage::{DirEntry, FileMetadata, Result, Storage, StorageError};

/// In-memory storage backend for testing.
///
/// Uses a thread-safe HashMap to store file contents in memory.
/// This backend is useful for unit testing without filesystem I/O.
pub struct MemoryStorage {
    data: Arc<RwLock<HashMap<PathBuf, Vec<u8>>>>,
    dirs: Arc<RwLock<HashMap<PathBuf, ()>>>,
}

impl MemoryStorage {
    /// Create a new in-memory storage instance.
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
            dirs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Clear all data from the storage.
    pub fn clear(&self) {
        self.data.write().unwrap().clear();
        self.dirs.write().unwrap().clear();
    }
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Storage for MemoryStorage {
    async fn exists(&self, path: &Path) -> bool {
        let data = self.data.read().unwrap();
        let dirs = self.dirs.read().unwrap();
        data.contains_key(path) || dirs.contains_key(path)
    }

    async fn is_file(&self, path: &Path) -> bool {
        self.data.read().unwrap().contains_key(path)
    }

    async fn is_dir(&self, path: &Path) -> bool {
        self.dirs.read().unwrap().contains_key(path)
    }

    async fn read(&self, path: &Path) -> Result<Vec<u8>> {
        let data = self.data.read().unwrap();
        data.get(path)
            .cloned()
            .ok_or_else(|| StorageError::NotFound(path.to_path_buf()))
    }

    async fn open(&self, _path: &Path) -> Result<tokio::fs::File> {
        // For in-memory storage, we can't return a real File
        // This is acceptable for testing - real file operations use NativeStorage
        Err(StorageError::Internal(
            "open() not supported for MemoryStorage - use NativeStorage for file streaming"
                .to_string(),
        ))
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> Result<()> {
        let mut data = self.data.write().unwrap();
        data.insert(path.to_path_buf(), contents.to_vec());
        Ok(())
    }

    async fn append(&self, path: &Path, contents: &[u8]) -> Result<()> {
        let mut data = self.data.write().unwrap();
        let mut existing = data.get(path).cloned().unwrap_or_default();
        existing.extend_from_slice(contents);
        data.insert(path.to_path_buf(), existing);
        Ok(())
    }

    async fn delete(&self, path: &Path) -> Result<()> {
        let mut data = self.data.write().unwrap();
        let mut dirs = self.dirs.write().unwrap();

        if data.remove(path).is_some() {
            return Ok(());
        }

        if dirs.remove(path).is_some() {
            // Also remove all children
            let prefix = format!("{}/", path.display());
            data.retain(|k, _| !k.starts_with(&prefix));
            dirs.retain(|k, _| !k.starts_with(&prefix));
            return Ok(());
        }

        Err(StorageError::NotFound(path.to_path_buf()))
    }

    async fn metadata(&self, path: &Path) -> Result<FileMetadata> {
        let data = self.data.read().unwrap();
        let dirs = self.dirs.read().unwrap();

        if let Some(contents) = data.get(path) {
            return Ok(FileMetadata::new(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                true,
                contents.len() as u64,
            ));
        }

        if dirs.contains_key(path) {
            return Ok(FileMetadata::new(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                false,
                0,
            ));
        }

        Err(StorageError::NotFound(path.to_path_buf()))
    }

    async fn list(&self, dir: &Path) -> Result<Vec<DirEntry>> {
        let data = self.data.read().unwrap();
        let dirs = self.dirs.read().unwrap();

        let mut entries = Vec::new();

        // Add directories
        for dir_path in dirs.keys() {
            if let Some(parent) = dir_path.parent()
                && parent == dir
            {
                entries.push(
                    DirEntry::new(
                        dir_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                        true,
                        0,
                    )
                    .with_mode(0o40755)
                    .with_modified(None),
                );
            }
        }

        // Add files
        for file_path in data.keys() {
            if let Some(parent) = file_path.parent()
                && parent == dir
            {
                entries.push(
                    DirEntry::new(
                        file_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                        false,
                        data.get(file_path).map_or(0, |v| v.len() as u64),
                    )
                    .with_mode(0o100644)
                    .with_modified(None),
                );
            }
        }

        // Sort for consistent ordering
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(entries)
    }

    async fn create_dir(&self, path: &Path) -> Result<()> {
        let mut dirs = self.dirs.write().unwrap();
        if dirs.contains_key(path) {
            return Err(StorageError::AlreadyExists(path.to_path_buf()));
        }
        if let Some(parent) = path.parent()
            && !dirs.contains_key(parent)
            && !self.dirs.read().unwrap().contains_key(parent)
        {
            return Err(StorageError::NotFound(parent.to_path_buf()));
        }
        dirs.insert(path.to_path_buf(), ());
        Ok(())
    }

    async fn create_dir_all(&self, path: &Path) -> Result<()> {
        let mut dirs = self.dirs.write().unwrap();
        let mut current = PathBuf::new();

        for component in path.components() {
            current.push(component);
            if !dirs.contains_key(&current) {
                dirs.insert(current.to_path_buf(), ());
            }
        }

        Ok(())
    }

    async fn remove_dir(&self, path: &Path) -> Result<()> {
        let mut dirs = self.dirs.write().unwrap();
        let data = self.data.read().unwrap();

        if !dirs.contains_key(path) {
            return Err(StorageError::NotFound(path.to_path_buf()));
        }

        // Check if directory is empty
        let prefix = format!("{}/", path.display());
        let has_children = data.keys().any(|k| k.starts_with(&prefix))
            || dirs.keys().any(|k| k.starts_with(&prefix) && k != path);

        if has_children {
            return Err(StorageError::DirectoryNotEmpty(path.to_path_buf()));
        }

        dirs.remove(path);
        Ok(())
    }

    async fn remove_dir_all(&self, path: &Path) -> Result<()> {
        let mut dirs = self.dirs.write().unwrap();
        let mut data = self.data.write().unwrap();

        let prefix = format!("{}/", path.display());

        // Remove all files and subdirectories
        data.retain(|k, _| !k.starts_with(&prefix));
        dirs.retain(|k, _| !k.starts_with(&prefix) && k != path);

        // Remove the directory itself
        dirs.remove(path);

        Ok(())
    }

    async fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let mut data = self.data.write().unwrap();
        let mut dirs = self.dirs.write().unwrap();

        if let Some(contents) = data.remove(from) {
            data.insert(to.to_path_buf(), contents);
            return Ok(());
        }

        if dirs.remove(from).is_some() {
            dirs.insert(to.to_path_buf(), ());
            return Ok(());
        }

        Err(StorageError::NotFound(from.to_path_buf()))
    }

    async fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        let data = self.data.read().unwrap();
        let contents = data
            .get(from)
            .cloned()
            .ok_or_else(|| StorageError::NotFound(from.to_path_buf()))?;

        let mut data_write = self.data.write().unwrap();
        data_write.insert(to.to_path_buf(), contents);
        Ok(())
    }

    async fn canonicalize(&self, path: &Path) -> Result<PathBuf> {
        // In memory storage, just return the canonical path
        Ok(path.to_path_buf())
    }

    async fn size(&self, path: &Path) -> Result<u64> {
        let data = self.data.read().unwrap();
        data.get(path)
            .map(|v| v.len() as u64)
            .ok_or_else(|| StorageError::NotFound(path.to_path_buf()))
    }
}

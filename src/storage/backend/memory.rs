use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::storage::{DirEntry, FileMetadata, Result, Storage, StorageError};

/// In-memory storage backend.
///
/// Uses a thread-safe `HashMap` to store file contents in memory.
/// This backend is useful for unit testing without filesystem I/O.
pub struct MemoryStorage {
    data: Arc<RwLock<HashMap<PathBuf, Vec<u8>>>>,
    dirs: Arc<RwLock<HashSet<PathBuf>>>,
}

impl MemoryStorage {
    /// Create a new in-memory storage instance.
    #[must_use]
    /// new
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
            dirs: Arc::new(RwLock::new(HashSet::new())),
        }
    }
}

impl Default for MemoryStorage {
    /// item
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Storage for MemoryStorage {
    async fn exists(&self, path: &Path) -> bool {
        let data = self.data.read().await;
        let dirs = self.dirs.read().await;
        data.contains_key(path)
            || dirs.contains(path)
            // A directory "exists" if any file path starts with it
            || data.keys().any(|k| k.starts_with(path) && k != path)
    }

    async fn is_file(&self, path: &Path) -> bool {
        self.data.read().await.contains_key(path)
    }

    async fn is_dir(&self, path: &Path) -> bool {
        self.dirs.read().await.contains(path)
    }

    async fn read(&self, path: &Path) -> Result<Vec<u8>> {
        let data = self.data.read().await;
        data.get(path)
            .cloned()
            .ok_or_else(|| StorageError::NotFound(path.to_path_buf()))
    }

    async fn open(&self, path: &Path) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let data = self.read(path).await?;
        Ok(Box::new(Cursor::new(data)))
    }

    async fn seek_read(
        &self,
        path: &Path,
        offset: u64,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let data = self.read(path).await?;
        if offset >= data.len() as u64 {
            return Ok(Box::new(Cursor::new(Vec::new())));
        }
        let sliced = data[offset as usize..].to_vec();
        Ok(Box::new(Cursor::new(sliced)))
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> Result<()> {
        let mut data = self.data.write().await;
        data.insert(path.to_path_buf(), contents.to_vec());
        Ok(())
    }

    async fn append(&self, path: &Path, contents: &[u8]) -> Result<()> {
        let mut data = self.data.write().await;
        let mut existing = data.get(path).cloned().unwrap_or_default();
        existing.extend_from_slice(contents);
        data.insert(path.to_path_buf(), existing);
        Ok(())
    }

    async fn delete(&self, path: &Path) -> Result<()> {
        let mut data = self.data.write().await;
        let mut dirs = self.dirs.write().await;

        if data.remove(path).is_some() {
            return Ok(());
        }

        if dirs.remove(path) {
            // Also remove all children
            let mut prefix = path.to_path_buf();
            prefix.push("");
            data.retain(|k, _| !k.starts_with(&prefix));
            dirs.retain(|k| !k.starts_with(&prefix));
            return Ok(());
        }

        Err(StorageError::NotFound(path.to_path_buf()))
    }

    async fn metadata(&self, path: &Path) -> Result<FileMetadata> {
        let data = self.data.read().await;
        let dirs = self.dirs.read().await;

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

        if dirs.contains(path) {
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
        let data = self.data.read().await;
        let dirs = self.dirs.read().await;

        let mut entries = Vec::new();

        // Add directories
        for dir_path in dirs.iter() {
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
                    .with_mode(0o100_644)
                    .with_modified(None),
                );
            }
        }

        // Sort for consistent ordering
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(entries)
    }

    async fn create_dir(&self, path: &Path) -> Result<()> {
        // Check if path already exists in dirs
        if self.dirs.read().await.contains(path) {
            return Err(StorageError::AlreadyExists(path.to_path_buf()));
        }
        // The root path "/" always implicitly exists.
        // For other paths, check if the parent directory exists (either in dirs or implied by files)
        if let Some(parent) = path.parent()
            && parent != Path::new("/")
        {
            let has_parent = {
                let dirs = self.dirs.read().await;
                let data = self.data.read().await;
                dirs.contains(parent) || data.keys().any(|k| k.starts_with(parent) && k != parent)
            };
            if !has_parent {
                return Err(StorageError::NotFound(parent.to_path_buf()));
            }
        }
        self.dirs.write().await.insert(path.to_path_buf());
        Ok(())
    }

    async fn create_dir_all(&self, path: &Path) -> Result<()> {
        let mut dirs = self.dirs.write().await;
        let mut current = PathBuf::new();

        for component in path.components() {
            current.push(component);
            if !dirs.contains(&current) {
                dirs.insert(current.clone());
            }
        }

        Ok(())
    }

    async fn remove_dir(&self, path: &Path) -> Result<()> {
        let mut dirs = self.dirs.write().await;
        let data = self.data.read().await;

        if !dirs.contains(path) {
            return Err(StorageError::NotFound(path.to_path_buf()));
        }

        // Check if directory is empty
        let mut prefix = path.to_path_buf();
        prefix.push("");
        let has_children = data.keys().any(|k| k.starts_with(&prefix))
            || dirs.iter().any(|k| k.starts_with(&prefix) && *k != path);

        if has_children {
            return Err(StorageError::DirectoryNotEmpty(path.to_path_buf()));
        }

        dirs.remove(path);
        Ok(())
    }

    async fn remove_dir_all(&self, path: &Path) -> Result<()> {
        let mut dirs = self.dirs.write().await;
        let mut data = self.data.write().await;

        let mut prefix = path.to_path_buf();
        prefix.push("");

        // Remove all files and subdirectories
        data.retain(|k, _| !k.starts_with(&prefix));
        dirs.retain(|k| !k.starts_with(&prefix) && k != path);

        // Remove the directory itself
        dirs.remove(path);

        Ok(())
    }

    async fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let mut data = self.data.write().await;
        let mut dirs = self.dirs.write().await;

        if let Some(contents) = data.remove(from) {
            data.insert(to.to_path_buf(), contents);
            return Ok(());
        }

        if dirs.remove(from) {
            dirs.insert(to.to_path_buf());
            return Ok(());
        }

        Err(StorageError::NotFound(from.to_path_buf()))
    }

    async fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        let contents = {
            let data = self.data.read().await;
            data.get(from)
                .cloned()
                .ok_or_else(|| StorageError::NotFound(from.to_path_buf()))?
        };
        // Read lock dropped above; now acquire write lock to insert
        self.data.write().await.insert(to.to_path_buf(), contents);
        Ok(())
    }

    async fn canonicalize(&self, path: &Path) -> Result<PathBuf> {
        // In memory storage, just return the canonical path
        Ok(path.to_path_buf())
    }

    async fn size(&self, path: &Path) -> Result<u64> {
        let data = self.data.read().await;
        data.get(path)
            .map(|v| v.len() as u64)
            .ok_or_else(|| StorageError::NotFound(path.to_path_buf()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn test_write_and_read() {
        let storage = MemoryStorage::new();
        let path = PathBuf::from("/test/hello.txt");
        let contents = b"Hello, world!";
        storage.write(&path, contents).await.unwrap();
        let result = storage.read(&path).await.unwrap();
        assert_eq!(result, contents);
    }

    #[tokio::test]
    async fn test_read_nonexistent() {
        let storage = MemoryStorage::new();
        let path = PathBuf::from("/nonexistent.txt");
        let result = storage.read(&path).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), StorageError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_exists() {
        let storage = MemoryStorage::new();
        let path = PathBuf::from("/exists.txt");
        assert!(!storage.exists(&path).await);
        storage.write(&path, b"data").await.unwrap();
        assert!(storage.exists(&path).await);
    }

    #[tokio::test]
    async fn test_is_file() {
        let storage = MemoryStorage::new();
        let path = PathBuf::from("/file.txt");
        storage.write(&path, b"data").await.unwrap();
        assert!(storage.is_file(&path).await);
        assert!(!storage.is_file(&PathBuf::from("/nonexistent.txt")).await);
    }

    #[tokio::test]
    async fn test_is_dir() {
        let storage = MemoryStorage::new();
        let path = PathBuf::from("/mydir");
        storage.create_dir(&path).await.unwrap();
        assert!(storage.is_dir(&path).await);
        assert!(!storage.is_dir(&PathBuf::from("/nonexistent")).await);
    }

    #[tokio::test]
    async fn test_write_overwrite() {
        let storage = MemoryStorage::new();
        let path = PathBuf::from("/overwrite.txt");
        storage.write(&path, b"first").await.unwrap();
        storage.write(&path, b"second").await.unwrap();
        let result = storage.read(&path).await.unwrap();
        assert_eq!(result, b"second");
    }

    #[tokio::test]
    async fn test_append_creates_file() {
        let storage = MemoryStorage::new();
        let path = PathBuf::from("/append_test.txt");
        storage.append(&path, b"part1").await.unwrap();
        storage.append(&path, b"part2").await.unwrap();
        let result = storage.read(&path).await.unwrap();
        assert_eq!(result, b"part1part2");
    }

    #[tokio::test]
    async fn test_append_existing_file() {
        let storage = MemoryStorage::new();
        let path = PathBuf::from("/append_existing.txt");
        storage.write(&path, b"first").await.unwrap();
        storage.append(&path, b"second").await.unwrap();
        let result = storage.read(&path).await.unwrap();
        assert_eq!(result, b"firstsecond");
    }

    #[tokio::test]
    async fn test_delete_file() {
        let storage = MemoryStorage::new();
        let path = PathBuf::from("/delete.txt");
        storage.write(&path, b"data").await.unwrap();
        storage.delete(&path).await.unwrap();
        assert!(!storage.exists(&path).await);
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let storage = MemoryStorage::new();
        let result = storage.delete(&PathBuf::from("/nonexistent.txt")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_dir_removes_children() {
        let storage = MemoryStorage::new();
        storage
            .write(&PathBuf::from("/dir/file1.txt"), b"data1")
            .await
            .unwrap();
        storage
            .write(&PathBuf::from("/dir/file2.txt"), b"data2")
            .await
            .unwrap();
        storage.create_dir(&PathBuf::from("/dir")).await.unwrap();
        storage.delete(&PathBuf::from("/dir")).await.unwrap();
        assert!(!storage.exists(&PathBuf::from("/dir")).await);
        assert!(!storage.exists(&PathBuf::from("/dir/file1.txt")).await);
        assert!(!storage.exists(&PathBuf::from("/dir/file2.txt")).await);
    }

    #[tokio::test]
    async fn test_metadata_file() {
        let storage = MemoryStorage::new();
        let path = PathBuf::from("/meta_file.txt");
        let contents = b"metadata test content";
        storage.write(&path, contents).await.unwrap();
        let meta = storage.metadata(&path).await.unwrap();
        assert!(meta.is_file);
        assert!(!meta.is_dir());
        assert_eq!(meta.size, contents.len() as u64);
        assert_eq!(meta.name, "meta_file.txt");
    }

    #[tokio::test]
    async fn test_metadata_dir() {
        let storage = MemoryStorage::new();
        let path = PathBuf::from("/meta_dir");
        storage.create_dir(&path).await.unwrap();
        let meta = storage.metadata(&path).await.unwrap();
        assert!(!meta.is_file);
        assert!(meta.is_dir());
        assert_eq!(meta.size, 0);
        assert_eq!(meta.name, "meta_dir");
    }

    #[tokio::test]
    async fn test_metadata_nonexistent() {
        let storage = MemoryStorage::new();
        let result = storage.metadata(&PathBuf::from("/nonexistent")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_dir() {
        let storage = MemoryStorage::new();
        storage.create_dir(&PathBuf::from("/mydir")).await.unwrap();
        assert!(storage.is_dir(&PathBuf::from("/mydir")).await);
    }

    #[tokio::test]
    async fn test_create_dir_already_exists() {
        let storage = MemoryStorage::new();
        storage.create_dir(&PathBuf::from("/mydir")).await.unwrap();
        let result = storage.create_dir(&PathBuf::from("/mydir")).await;
        assert!(matches!(
            result.unwrap_err(),
            StorageError::AlreadyExists(_)
        ));
    }

    #[tokio::test]
    async fn test_create_dir_no_parent() {
        let storage = MemoryStorage::new();
        let result = storage.create_dir(&PathBuf::from("/nonexistent/dir")).await;
        assert!(matches!(result.unwrap_err(), StorageError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_create_dir_all() {
        let storage = MemoryStorage::new();
        storage
            .create_dir_all(&PathBuf::from("/a/b/c"))
            .await
            .unwrap();
        assert!(storage.is_dir(&PathBuf::from("/a")).await);
        assert!(storage.is_dir(&PathBuf::from("/a/b")).await);
        assert!(storage.is_dir(&PathBuf::from("/a/b/c")).await);
    }

    #[tokio::test]
    async fn test_remove_dir_empty() {
        let storage = MemoryStorage::new();
        storage
            .create_dir(&PathBuf::from("/empty_dir"))
            .await
            .unwrap();
        storage
            .remove_dir(&PathBuf::from("/empty_dir"))
            .await
            .unwrap();
        assert!(!storage.exists(&PathBuf::from("/empty_dir")).await);
    }

    #[tokio::test]
    async fn test_remove_dir_not_empty() {
        let storage = MemoryStorage::new();
        storage.create_dir(&PathBuf::from("/dir")).await.unwrap();
        storage
            .write(&PathBuf::from("/dir/file.txt"), b"data")
            .await
            .unwrap();
        let result = storage.remove_dir(&PathBuf::from("/dir")).await;
        assert!(matches!(
            result.unwrap_err(),
            StorageError::DirectoryNotEmpty(_)
        ));
    }

    #[tokio::test]
    async fn test_remove_dir_nonexistent() {
        let storage = MemoryStorage::new();
        let result = storage.remove_dir(&PathBuf::from("/nonexistent")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_remove_dir_all() {
        let storage = MemoryStorage::new();
        storage
            .write(&PathBuf::from("/dir/file1.txt"), b"data1")
            .await
            .unwrap();
        storage
            .write(&PathBuf::from("/dir/file2.txt"), b"data2")
            .await
            .unwrap();
        storage
            .create_dir(&PathBuf::from("/dir/subdir"))
            .await
            .unwrap();
        storage
            .write(&PathBuf::from("/dir/subdir/file3.txt"), b"data3")
            .await
            .unwrap();
        storage
            .remove_dir_all(&PathBuf::from("/dir"))
            .await
            .unwrap();
        assert!(!storage.exists(&PathBuf::from("/dir")).await);
        assert!(!storage.exists(&PathBuf::from("/dir/file1.txt")).await);
        assert!(!storage.exists(&PathBuf::from("/dir/subdir")).await);
    }

    #[tokio::test]
    async fn test_rename_file() {
        let storage = MemoryStorage::new();
        let from = PathBuf::from("/old.txt");
        let to = PathBuf::from("/new.txt");
        storage.write(&from, b"data").await.unwrap();
        storage.rename(&from, &to).await.unwrap();
        assert!(!storage.exists(&from).await);
        assert!(storage.exists(&to).await);
        assert_eq!(storage.read(&to).await.unwrap(), b"data");
    }

    #[tokio::test]
    async fn test_rename_dir() {
        let storage = MemoryStorage::new();
        let from = PathBuf::from("/olddir");
        let to = PathBuf::from("/newdir");
        storage.create_dir(&from).await.unwrap();
        storage.rename(&from, &to).await.unwrap();
        assert!(!storage.exists(&from).await);
        assert!(storage.is_dir(&to).await);
    }

    #[tokio::test]
    async fn test_rename_nonexistent() {
        let storage = MemoryStorage::new();
        let result = storage
            .rename(
                &PathBuf::from("/nonexistent.txt"),
                &PathBuf::from("/new.txt"),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_copy_file() {
        let storage = MemoryStorage::new();
        let from = PathBuf::from("/source.txt");
        let to = PathBuf::from("/dest.txt");
        storage.write(&from, b"copy me").await.unwrap();
        storage.copy(&from, &to).await.unwrap();
        assert!(storage.exists(&to).await);
        assert_eq!(storage.read(&to).await.unwrap(), b"copy me");
        assert_eq!(storage.read(&from).await.unwrap(), b"copy me");
    }

    #[tokio::test]
    async fn test_copy_nonexistent_source() {
        let storage = MemoryStorage::new();
        let result = storage
            .copy(
                &PathBuf::from("/nonexistent.txt"),
                &PathBuf::from("/dest.txt"),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_canonicalize() {
        let storage = MemoryStorage::new();
        let path = PathBuf::from("/foo/bar/baz.txt");
        let result = storage.canonicalize(&path).await.unwrap();
        assert_eq!(result, path);
    }

    #[tokio::test]
    async fn test_size() {
        let storage = MemoryStorage::new();
        let path = PathBuf::from("/size_test.txt");
        let contents = b"12345";
        storage.write(&path, contents).await.unwrap();
        let size = storage.size(&path).await.unwrap();
        assert_eq!(size, 5);
    }

    #[tokio::test]
    async fn test_size_nonexistent() {
        let storage = MemoryStorage::new();
        let result = storage.size(&PathBuf::from("/nonexistent.txt")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_empty_dir() {
        let storage = MemoryStorage::new();
        storage.create_dir(&PathBuf::from("/empty")).await.unwrap();
        let entries = storage.list(&PathBuf::from("/empty")).await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_list_files_and_dirs() {
        let storage = MemoryStorage::new();
        storage
            .create_dir(&PathBuf::from("/list_dir"))
            .await
            .unwrap();
        storage
            .write(&PathBuf::from("/list_dir/file1.txt"), b"data")
            .await
            .unwrap();
        storage
            .write(&PathBuf::from("/list_dir/file2.txt"), b"data")
            .await
            .unwrap();
        storage
            .create_dir(&PathBuf::from("/list_dir/subdir"))
            .await
            .unwrap();
        let entries = storage.list(&PathBuf::from("/list_dir")).await.unwrap();
        assert_eq!(entries.len(), 3);
        // Verify sorted order
        assert_eq!(entries[0].name, "file1.txt");
        assert_eq!(entries[1].name, "file2.txt");
        assert_eq!(entries[2].name, "subdir");
        // Check modes
        assert_eq!(entries[0].mode, 0o100_644);
        assert_eq!(entries[2].mode, 0o40755);
    }

    #[tokio::test]
    async fn test_list_sorted() {
        let storage = MemoryStorage::new();
        storage
            .write(&PathBuf::from("/sort/z.txt"), b"a")
            .await
            .unwrap();
        storage
            .write(&PathBuf::from("/sort/a.txt"), b"b")
            .await
            .unwrap();
        storage
            .write(&PathBuf::from("/sort/m.txt"), b"c")
            .await
            .unwrap();
        let entries = storage.list(&PathBuf::from("/sort")).await.unwrap();
        assert_eq!(entries[0].name, "a.txt");
        assert_eq!(entries[1].name, "m.txt");
        assert_eq!(entries[2].name, "z.txt");
    }

    #[tokio::test]
    async fn test_default() {
        let storage = MemoryStorage::default();
        let path = PathBuf::from("/default.txt");
        storage.write(&path, b"test").await.unwrap();
        assert!(storage.exists(&path).await);
    }

    #[tokio::test]
    async fn test_open_returns_streaming_reader() {
        let storage = MemoryStorage::new();
        let path = PathBuf::from("/open_test.txt");
        storage.write(&path, b"open test data").await.unwrap();
        let mut reader = storage.open(&path).await.unwrap();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).await.unwrap();
        assert_eq!(buf, b"open test data");
    }
}

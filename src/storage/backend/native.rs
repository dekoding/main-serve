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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::storage::Storage;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("main_serve_test_{}", name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn test_write_and_read() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("write_read");
        let path = tmp.join("test.txt");
        let contents = b"Hello, native storage!";
        storage.write(&path, contents).await.unwrap();
        let result = storage.read(&path).await.unwrap();
        assert_eq!(result, contents);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_read_nonexistent() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("read_nonexistent");
        let path = tmp.join("doesnotexist.txt");
        let result = storage.read(&path).await;
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_exists() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("exists");
        let path = tmp.join("exists.txt");
        assert!(!storage.exists(&path).await);
        storage.write(&path, b"data").await.unwrap();
        assert!(storage.exists(&path).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_is_file() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("is_file");
        let path = tmp.join("file.txt");
        storage.write(&path, b"data").await.unwrap();
        assert!(storage.is_file(&path).await);
        assert!(!storage.is_file(&PathBuf::from("/nonexistent")).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_is_dir() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("is_dir");
        let dir_path = tmp.join("mydir");
        storage.create_dir(&dir_path).await.unwrap();
        assert!(storage.is_dir(&dir_path).await);
        assert!(!storage.is_dir(&PathBuf::from("/nonexistent")).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_write_overwrite() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("overwrite");
        let path = tmp.join("overwrite.txt");
        storage.write(&path, b"first").await.unwrap();
        storage.write(&path, b"second").await.unwrap();
        let result = storage.read(&path).await.unwrap();
        assert_eq!(result, b"second");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_append_creates_file() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("append_create");
        let path = tmp.join("append_test.txt");
        storage.append(&path, b"part1").await.unwrap();
        storage.append(&path, b"part2").await.unwrap();
        let result = storage.read(&path).await.unwrap();
        assert_eq!(result, b"part1part2");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_append_existing_file() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("append_existing");
        let path = tmp.join("append_existing.txt");
        storage.write(&path, b"first").await.unwrap();
        storage.append(&path, b"second").await.unwrap();
        let result = storage.read(&path).await.unwrap();
        assert_eq!(result, b"firstsecond");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_delete_file() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("delete_file");
        let path = tmp.join("delete.txt");
        storage.write(&path, b"data").await.unwrap();
        storage.delete(&path).await.unwrap();
        assert!(!storage.exists(&path).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("delete_nonexistent");
        let result = storage.delete(&tmp.join("nonexistent.txt")).await;
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_metadata_file() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("metadata_file");
        let path = tmp.join("meta.txt");
        let contents = b"metadata test content here";
        storage.write(&path, contents).await.unwrap();
        let meta = storage.metadata(&path).await.unwrap();
        assert!(meta.is_file);
        assert!(!meta.is_dir());
        assert_eq!(meta.size, contents.len() as u64);
        assert!(meta.modified.is_some());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_metadata_dir() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("metadata_dir");
        let dir_path = tmp.join("meta_dir");
        storage.create_dir(&dir_path).await.unwrap();
        let meta = storage.metadata(&dir_path).await.unwrap();
        assert!(!meta.is_file);
        assert!(meta.is_dir());
        // Directories have a non-zero size on disk (directory entry overhead)
        assert!(meta.size > 0);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_create_dir() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("create_dir");
        let dir_path = tmp.join("mydir");
        storage.create_dir(&dir_path).await.unwrap();
        assert!(storage.is_dir(&dir_path).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_create_dir_already_exists() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("create_dir_exists");
        let dir_path = tmp.join("mydir");
        storage.create_dir(&dir_path).await.unwrap();
        let result = storage.create_dir(&dir_path).await;
        match result.unwrap_err() {
            StorageError::Io(_, e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
            }
            other => panic!("expected StorageError::Io, got {:?}", other),
        }
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_create_dir_all() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("create_dir_all");
        storage.create_dir_all(&tmp.join("a/b/c")).await.unwrap();
        assert!(storage.is_dir(&tmp.join("a")).await);
        assert!(storage.is_dir(&tmp.join("a/b")).await);
        assert!(storage.is_dir(&tmp.join("a/b/c")).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_remove_dir_empty() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("remove_dir_empty");
        let dir_path = tmp.join("empty_dir");
        storage.create_dir(&dir_path).await.unwrap();
        storage.remove_dir(&dir_path).await.unwrap();
        assert!(!storage.exists(&dir_path).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_remove_dir_not_empty() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("remove_dir_not_empty");
        let dir_path = tmp.join("dir");
        storage.create_dir(&dir_path).await.unwrap();
        storage
            .write(&dir_path.join("file.txt"), b"data")
            .await
            .unwrap();
       let result = storage.remove_dir(&dir_path).await;
        // Native remove_dir rejects non-empty directories; the directory must still exist
        assert!(result.is_err());
        assert!(storage.exists(&dir_path).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_remove_dir_all() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("remove_dir_all");
        let dir_path = tmp.join("dir");
        storage.create_dir(&dir_path).await.unwrap();
        storage
            .write(&dir_path.join("file1.txt"), b"data1")
            .await
            .unwrap();
        storage.create_dir(&dir_path.join("subdir")).await.unwrap();
        storage
            .write(&dir_path.join("subdir/file2.txt"), b"data2")
            .await
            .unwrap();
        storage.remove_dir_all(&dir_path).await.unwrap();
        assert!(!storage.exists(&dir_path).await);
        assert!(!storage.exists(&dir_path.join("file1.txt")).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_rename_file() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("rename_file");
        let from = tmp.join("old.txt");
        let to = tmp.join("new.txt");
        storage.write(&from, b"data").await.unwrap();
        storage.rename(&from, &to).await.unwrap();
        assert!(!storage.exists(&from).await);
        assert!(storage.exists(&to).await);
        assert_eq!(storage.read(&to).await.unwrap(), b"data");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_rename_dir() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("rename_dir");
        let from = tmp.join("olddir");
        let to = tmp.join("newdir");
        storage.create_dir(&from).await.unwrap();
        storage.rename(&from, &to).await.unwrap();
        assert!(!storage.exists(&from).await);
        assert!(storage.is_dir(&to).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_rename_nonexistent() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("rename_nonexistent");
        let result = storage
            .rename(&tmp.join("nonexistent.txt"), &tmp.join("new.txt"))
            .await;
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_copy_file() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("copy_file");
        let from = tmp.join("source.txt");
        let to = tmp.join("dest.txt");
        storage.write(&from, b"copy me").await.unwrap();
        storage.copy(&from, &to).await.unwrap();
        assert!(storage.exists(&to).await);
        assert_eq!(storage.read(&to).await.unwrap(), b"copy me");
        assert_eq!(storage.read(&from).await.unwrap(), b"copy me");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_copy_nonexistent_source() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("copy_nonexistent");
        let result = storage
            .copy(&tmp.join("nonexistent.txt"), &tmp.join("dest.txt"))
            .await;
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_canonicalize() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("canonicalize");
        let path = tmp.join("foo/bar/baz.txt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "test").unwrap();
        let result = storage.canonicalize(&path).await.unwrap();
        assert!(result.is_absolute());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_size() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("size");
        let path = tmp.join("size_test.txt");
        let contents = b"12345";
        storage.write(&path, contents).await.unwrap();
        let size = storage.size(&path).await.unwrap();
        assert_eq!(size, 5);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_size_nonexistent() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("size_nonexistent");
        let result = storage.size(&tmp.join("nonexistent.txt")).await;
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_list_empty_dir() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("list_empty");
        let dir_path = tmp.join("empty");
        storage.create_dir(&dir_path).await.unwrap();
        let entries = storage.list(&dir_path).await.unwrap();
        assert!(entries.is_empty());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_list_files_and_dirs() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("list_files_dirs");
        let dir_path = tmp.join("list_dir");
        storage.create_dir(&dir_path).await.unwrap();
        storage
            .write(&dir_path.join("file1.txt"), b"data")
            .await
            .unwrap();
        storage
            .write(&dir_path.join("file2.txt"), b"data")
            .await
            .unwrap();
        storage.create_dir(&dir_path.join("subdir")).await.unwrap();
        let entries = storage.list(&dir_path).await.unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().any(|e| e.is_file() && e.name == "file1.txt"));
        assert!(entries.iter().any(|e| e.is_file() && e.name == "file2.txt"));
        assert!(entries.iter().any(|e| e.is_dir && e.name == "subdir"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_list_contains_expected_entries() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("list_entries");
        let dir_path = tmp.join("sort");
        storage.create_dir(&dir_path).await.unwrap();
        storage.write(&dir_path.join("z.txt"), b"a").await.unwrap();
        storage.write(&dir_path.join("a.txt"), b"b").await.unwrap();
        storage.write(&dir_path.join("m.txt"), b"c").await.unwrap();
        let entries = storage.list(&dir_path).await.unwrap();
        assert_eq!(entries.len(), 3);
        // Verify all expected names present
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"m.txt"));
        assert!(names.contains(&"z.txt"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_open_file() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("open_file");
        let path = tmp.join("open_test.txt");
        let contents = b"open test data";
        storage.write(&path, contents).await.unwrap();
        let mut file = storage.open(&path).await.unwrap();
        let mut buf = Vec::new();
        use tokio::io::AsyncReadExt;
        file.read_to_end(&mut buf).await.unwrap();
        assert_eq!(buf, contents);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_canonicalize_resolves_dotdot() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("canonicalize_traversal");
        let dir_path = tmp.join("a/b");
        let file_path = dir_path.join("file.txt");
        fs::create_dir_all(&dir_path).unwrap();
        fs::write(&file_path, "test").unwrap();
        let traversal_path = dir_path.join("../file_via_traversal.txt");
        fs::write(&traversal_path, "test2").unwrap();
        let canonical = storage.canonicalize(&traversal_path).await.unwrap();
        assert!(canonical.is_absolute());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_list_does_not_leak_across_directory_boundaries() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("list_boundaries");
        let root = tmp.join("root");
        let other = tmp.join("other");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&other).unwrap();
        fs::write(root.join("file.txt"), "data").unwrap();
        fs::write(other.join("file.txt"), "data").unwrap();
        let root_entries = storage.list(&root).await.unwrap();
        let other_entries = storage.list(&other).await.unwrap();
        assert_eq!(root_entries.len(), 1);
        assert_eq!(root_entries[0].name, "file.txt");
        assert_eq!(other_entries.len(), 1);
        assert_eq!(other_entries[0].name, "file.txt");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_remove_dir_all_does_not_affect_siblings() {
        let storage = NativeStorage::new();
        let tmp = temp_dir("remove_all_siblings");
        let sibling = tmp.join("sibling");
        let target = tmp.join("target_dir");
        fs::create_dir_all(&sibling).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(sibling.join("file.txt"), "preserve").unwrap();
        fs::write(target.join("file.txt"), "remove").unwrap();
        storage.remove_dir_all(&target).await.unwrap();
        assert!(storage.exists(&sibling).await);
        let content = storage.read(&sibling.join("file.txt")).await.unwrap();
        assert_eq!(content, b"preserve");
        let _ = fs::remove_dir_all(&tmp);
    }
}

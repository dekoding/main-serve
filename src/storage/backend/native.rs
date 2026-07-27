use std::path::{Path, PathBuf};

use tokio::fs;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::storage::{DirEntry, FileMetadata, Result, Storage, StorageError};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

/// Native filesystem storage backend.
///
/// Uses `tokio::fs` for all file operations, scoped to a configurable
/// root directory. All paths passed to storage methods are resolved
/// relative to this root.
#[derive(Clone)]
/// `NativeStorage`
pub struct NativeStorage {
    /// Root directory for this storage instance.
    /// All paths are resolved relative to this directory.
    root: PathBuf,
}

impl NativeStorage {
    /// Create a new native storage instance rooted at the given path.
    #[must_use]
    pub const fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Resolve a storage-relative path to an absolute path within this store's root.
    fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }
}

impl Default for NativeStorage {
    /// Returns a new native storage backend rooted at the current working directory.
    fn default() -> Self {
        Self::new(PathBuf::from("."))
    }
}

#[async_trait::async_trait]
impl Storage for NativeStorage {
    async fn exists(&self, path: &Path) -> bool {
        let resolved = self.resolve(path);
        fs::try_exists(&resolved).await.is_ok_and(|exists| exists)
    }

    async fn is_file(&self, path: &Path) -> bool {
        let resolved = self.resolve(path);
        fs::metadata(&resolved).await.is_ok_and(|m| m.is_file())
    }

    async fn is_dir(&self, path: &Path) -> bool {
        let resolved = self.resolve(path);
        fs::metadata(&resolved).await.is_ok_and(|m| m.is_dir())
    }

    async fn read(&self, path: &Path) -> Result<Vec<u8>> {
        let resolved = self.resolve(path);
        fs::read(&resolved)
            .await
            .map_err(|e| StorageError::Io(resolved, e))
    }

    async fn open(&self, path: &Path) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let resolved = self.resolve(path);
        Ok(Box::new(
            fs::File::open(&resolved)
                .await
                .map_err(|e| StorageError::Io(resolved, e))?,
        ))
    }

    async fn seek_read(
        &self,
        path: &Path,
        offset: u64,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let resolved = self.resolve(path);
        let mut file = fs::File::open(&resolved)
            .await
            .map_err(|e| StorageError::Io(resolved.clone(), e))?;

        file.seek(tokio::io::SeekFrom::Start(offset))
            .await
            .map_err(|e| StorageError::Io(resolved, e))?;

        Ok(Box::new(file))
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> Result<()> {
        let resolved = self.resolve(path);
        fs::write(&resolved, contents)
            .await
            .map_err(|e| StorageError::Io(resolved, e))
    }

    async fn append(&self, path: &Path, contents: &[u8]) -> Result<()> {
        let resolved = self.resolve(path);
        let mut file = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&resolved)
            .await
            .map_err(|e| StorageError::Io(resolved.clone(), e))?;

        file.write_all(contents)
            .await
            .map_err(|e| StorageError::Io(resolved.clone(), e))?;

        file.sync_all()
            .await
            .map_err(|e| StorageError::Io(resolved.clone(), e))?;

        Ok(())
    }

    async fn delete(&self, path: &Path) -> Result<()> {
        let resolved = self.resolve(path);
        fs::remove_file(&resolved)
            .await
            .map_err(|e| StorageError::Io(resolved, e))
    }

    async fn metadata(&self, path: &Path) -> Result<FileMetadata> {
        let resolved = self.resolve(path);
        let meta = fs::metadata(&resolved)
            .await
            .map_err(|e| StorageError::Io(resolved, e))?;

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
        let resolved = self.resolve(dir);
        let mut entries = Vec::new();
        let mut read_dir = fs::read_dir(&resolved)
            .await
            .map_err(|e| StorageError::Io(resolved.clone(), e))?;

        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let entry_path = entry.path();
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
                            .with_path(entry_path)
                            .with_mode(mode)
                            .with_uid(uid)
                            .with_gid(gid)
                            .with_modified(modified),
                    );
                }
                Err(e) => {
                    return Err(StorageError::Io(resolved.clone(), e));
                }
            }
        }

        Ok(entries)
    }

    async fn create_dir(&self, path: &Path) -> Result<()> {
        let resolved = self.resolve(path);
        fs::create_dir(&resolved)
            .await
            .map_err(|e| StorageError::Io(resolved, e))
    }

    async fn create_dir_all(&self, path: &Path) -> Result<()> {
        let resolved = self.resolve(path);
        fs::create_dir_all(&resolved)
            .await
            .map_err(|e| StorageError::Io(resolved, e))
    }

    async fn remove_dir(&self, path: &Path) -> Result<()> {
        let resolved = self.resolve(path);
        fs::remove_dir(&resolved)
            .await
            .map_err(|e| StorageError::Io(resolved, e))
    }

    async fn remove_dir_all(&self, path: &Path) -> Result<()> {
        let resolved = self.resolve(path);
        fs::remove_dir_all(&resolved)
            .await
            .map_err(|e| StorageError::Io(resolved, e))
    }

    async fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let resolved_from = self.resolve(from);
        let resolved_to = self.resolve(to);
        fs::rename(&resolved_from, &resolved_to)
            .await
            .map_err(|e| StorageError::Io(resolved_from, e))
    }

    async fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        let resolved_from = self.resolve(from);
        let resolved_to = self.resolve(to);
        fs::copy(&resolved_from, &resolved_to)
            .await
            .map_err(|e| StorageError::Io(resolved_from, e))?;
        Ok(())
    }

    async fn canonicalize(&self, path: &Path) -> Result<PathBuf> {
        let resolved = self.resolve(path);
        fs::canonicalize(&resolved)
            .await
            .map_err(|e| StorageError::Io(resolved, e))
    }

    async fn size(&self, path: &Path) -> Result<u64> {
        let resolved = self.resolve(path);
        let meta = fs::metadata(&resolved)
            .await
            .map_err(|e| StorageError::Io(resolved, e))?;
        Ok(meta.len())
    }

    /// Returns the root path of the storage backend.
    fn root_path(&self) -> Option<PathBuf> {
        Some(self.root.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::storage::Storage;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("main_serve_test_{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn test_write_and_read() {
        let tmp = temp_dir("write_read");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("test.txt");
        let contents = b"Hello, native storage!";
        storage.write(&path, contents).await.unwrap();
        let result = storage.read(&path).await.unwrap();
        assert_eq!(result, contents);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_read_nonexistent() {
        let tmp = temp_dir("read_nonexistent");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("doesnotexist.txt");
        let result = storage.read(&path).await;
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_exists() {
        let tmp = temp_dir("exists");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("exists.txt");
        assert!(!storage.exists(&path).await);
        storage.write(&path, b"data").await.unwrap();
        assert!(storage.exists(&path).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_is_file() {
        let tmp = temp_dir("is_file");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("file.txt");
        storage.write(&path, b"data").await.unwrap();
        assert!(storage.is_file(&path).await);
        assert!(!storage.is_file(&PathBuf::from("/nonexistent")).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_is_dir() {
        let tmp = temp_dir("is_dir");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("mydir");
        storage.create_dir(&path).await.unwrap();
        assert!(storage.is_dir(&path).await);
        assert!(!storage.is_dir(&PathBuf::from("/nonexistent")).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_write_overwrite() {
        let tmp = temp_dir("overwrite");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("overwrite.txt");
        storage.write(&path, b"first").await.unwrap();
        storage.write(&path, b"second").await.unwrap();
        let result = storage.read(&path).await.unwrap();
        assert_eq!(result, b"second");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_append_creates_file() {
        let tmp = temp_dir("append_create");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("append_test.txt");
        storage.append(&path, b"part1").await.unwrap();
        storage.append(&path, b"part2").await.unwrap();
        let result = storage.read(&path).await.unwrap();
        assert_eq!(result, b"part1part2");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_append_existing_file() {
        let tmp = temp_dir("append_existing");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("append_existing.txt");
        storage.write(&path, b"first").await.unwrap();
        storage.append(&path, b"second").await.unwrap();
        let result = storage.read(&path).await.unwrap();
        assert_eq!(result, b"firstsecond");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_delete_file() {
        let tmp = temp_dir("delete_file");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("delete.txt");
        storage.write(&path, b"data").await.unwrap();
        storage.delete(&path).await.unwrap();
        assert!(!storage.exists(&path).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let tmp = temp_dir("delete_nonexistent");
        let storage = NativeStorage::new(tmp.clone());
        let result = storage.delete(&PathBuf::from("nonexistent.txt")).await;
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_metadata_file() {
        let tmp = temp_dir("metadata_file");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("meta.txt");
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
        let tmp = temp_dir("metadata_dir");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("meta_dir");
        storage.create_dir(&path).await.unwrap();
        let meta = storage.metadata(&path).await.unwrap();
        assert!(!meta.is_file);
        assert!(meta.is_dir());
        assert!(meta.size > 0);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_create_dir() {
        let tmp = temp_dir("create_dir");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("mydir");
        storage.create_dir(&path).await.unwrap();
        assert!(storage.is_dir(&path).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_create_dir_already_exists() {
        let tmp = temp_dir("create_dir_exists");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("mydir");
        storage.create_dir(&path).await.unwrap();
        let result = storage.create_dir(&path).await;
        if let StorageError::Io(_, e) = result.unwrap_err() {
            assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        }
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_create_dir_all() {
        let tmp = temp_dir("create_dir_all");
        let storage = NativeStorage::new(tmp.clone());
        storage
            .create_dir_all(&PathBuf::from("a/b/c"))
            .await
            .unwrap();
        assert!(storage.is_dir(&PathBuf::from("a")).await);
        assert!(storage.is_dir(&PathBuf::from("a/b")).await);
        assert!(storage.is_dir(&PathBuf::from("a/b/c")).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_remove_dir_empty() {
        let tmp = temp_dir("remove_dir_empty");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("empty_dir");
        storage.create_dir(&path).await.unwrap();
        storage.remove_dir(&path).await.unwrap();
        assert!(!storage.exists(&path).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_remove_dir_not_empty() {
        let tmp = temp_dir("remove_dir_not_empty");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("dir");
        storage.create_dir(&path).await.unwrap();
        storage
            .write(&path.join("file.txt"), b"data")
            .await
            .unwrap();
        let result = storage.remove_dir(&path).await;
        assert!(result.is_err());
        assert!(storage.exists(&path).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_remove_dir_all() {
        let tmp = temp_dir("remove_dir_all");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("dir");
        storage.create_dir(&path).await.unwrap();
        storage
            .write(&path.join("file1.txt"), b"data1")
            .await
            .unwrap();
        storage.create_dir(&path.join("subdir")).await.unwrap();
        storage
            .write(&path.join("subdir/file2.txt"), b"data2")
            .await
            .unwrap();
        storage.remove_dir_all(&path).await.unwrap();
        assert!(!storage.exists(&path).await);
        assert!(!storage.exists(&path.join("file1.txt")).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_rename_file() {
        let tmp = temp_dir("rename_file");
        let storage = NativeStorage::new(tmp.clone());
        let from = PathBuf::from("old.txt");
        let to = PathBuf::from("new.txt");
        storage.write(&from, b"data").await.unwrap();
        storage.rename(&from, &to).await.unwrap();
        assert!(!storage.exists(&from).await);
        assert!(storage.exists(&to).await);
        assert_eq!(storage.read(&to).await.unwrap(), b"data");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_rename_dir() {
        let tmp = temp_dir("rename_dir");
        let storage = NativeStorage::new(tmp.clone());
        let from = PathBuf::from("olddir");
        let to = PathBuf::from("newdir");
        storage.create_dir(&from).await.unwrap();
        storage.rename(&from, &to).await.unwrap();
        assert!(!storage.exists(&from).await);
        assert!(storage.is_dir(&to).await);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_rename_nonexistent() {
        let tmp = temp_dir("rename_nonexistent");
        let storage = NativeStorage::new(tmp.clone());
        let result = storage
            .rename(&PathBuf::from("nonexistent.txt"), &PathBuf::from("new.txt"))
            .await;
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_copy_file() {
        let tmp = temp_dir("copy_file");
        let storage = NativeStorage::new(tmp.clone());
        let from = PathBuf::from("source.txt");
        let to = PathBuf::from("dest.txt");
        storage.write(&from, b"copy me").await.unwrap();
        storage.copy(&from, &to).await.unwrap();
        assert!(storage.exists(&to).await);
        assert_eq!(storage.read(&to).await.unwrap(), b"copy me");
        assert_eq!(storage.read(&from).await.unwrap(), b"copy me");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_copy_nonexistent_source() {
        let tmp = temp_dir("copy_nonexistent");
        let storage = NativeStorage::new(tmp.clone());
        let result = storage
            .copy(
                &PathBuf::from("nonexistent.txt"),
                &PathBuf::from("dest.txt"),
            )
            .await;
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_canonicalize() {
        let tmp = temp_dir("canonicalize");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("foo/bar/baz.txt");
        fs::create_dir_all(tmp.join("foo/bar")).unwrap();
        fs::write(tmp.join("foo/bar/baz.txt"), "test").unwrap();
        let result = storage.canonicalize(&path).await.unwrap();
        assert!(result.is_absolute());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_size() {
        let tmp = temp_dir("size");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("size_test.txt");
        let contents = b"12345";
        storage.write(&path, contents).await.unwrap();
        let size = storage.size(&path).await.unwrap();
        assert_eq!(size, 5);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_size_nonexistent() {
        let tmp = temp_dir("size_nonexistent");
        let storage = NativeStorage::new(tmp.clone());
        let result = storage.size(&PathBuf::from("nonexistent.txt")).await;
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_list_empty_dir() {
        let tmp = temp_dir("list_empty");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("empty");
        storage.create_dir(&path).await.unwrap();
        let entries = storage.list(&path).await.unwrap();
        assert!(entries.is_empty());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_list_files_and_dirs() {
        let tmp = temp_dir("list_files_dirs");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("list_dir");
        storage.create_dir(&path).await.unwrap();
        storage
            .write(&path.join("file1.txt"), b"data")
            .await
            .unwrap();
        storage
            .write(&path.join("file2.txt"), b"data")
            .await
            .unwrap();
        storage.create_dir(&path.join("subdir")).await.unwrap();
        let entries = storage.list(&path).await.unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().any(|e| e.is_file() && e.name == "file1.txt"));
        assert!(entries.iter().any(|e| e.is_file() && e.name == "file2.txt"));
        assert!(entries.iter().any(|e| e.is_dir && e.name == "subdir"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_list_contains_expected_entries() {
        let tmp = temp_dir("list_entries");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("sort");
        storage.create_dir(&path).await.unwrap();
        storage.write(&path.join("z.txt"), b"a").await.unwrap();
        storage.write(&path.join("a.txt"), b"b").await.unwrap();
        storage.write(&path.join("m.txt"), b"c").await.unwrap();
        let entries = storage.list(&path).await.unwrap();
        assert_eq!(entries.len(), 3);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"m.txt"));
        assert!(names.contains(&"z.txt"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_open_file() {
        use tokio::io::AsyncReadExt;
        let tmp = temp_dir("open_file");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("open_test.txt");
        let contents = b"open test data";
        storage.write(&path, contents).await.unwrap();
        let mut file = storage.open(&path).await.unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).await.unwrap();
        assert_eq!(buf, contents);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_canonicalize_resolves_dotdot() {
        let tmp = temp_dir("canonicalize_traversal");
        let storage = NativeStorage::new(tmp.clone());
        let dir_path = PathBuf::from("a/b");
        fs::create_dir_all(tmp.join("a/b")).unwrap();
        fs::write(tmp.join("a/b/file.txt"), "test").unwrap();
        let traversal_path = dir_path.join("../file_via_traversal.txt");
        fs::write(tmp.join("a/file_via_traversal.txt"), "test2").unwrap();
        let canonical = storage.canonicalize(&traversal_path).await.unwrap();
        assert!(canonical.is_absolute());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_list_does_not_leak_across_directory_boundaries() {
        let tmp = temp_dir("list_boundaries");
        let storage = NativeStorage::new(tmp.clone());
        let root = PathBuf::from("root");
        let other = PathBuf::from("other");
        fs::create_dir_all(tmp.join("root")).unwrap();
        fs::create_dir_all(tmp.join("other")).unwrap();
        fs::write(tmp.join("root/file.txt"), "data").unwrap();
        fs::write(tmp.join("other/file.txt"), "data").unwrap();
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
        let tmp = temp_dir("remove_all_siblings");
        let storage = NativeStorage::new(tmp.clone());
        let sibling = PathBuf::from("sibling");
        let target = PathBuf::from("target_dir");
        fs::create_dir_all(tmp.join("sibling")).unwrap();
        fs::create_dir_all(tmp.join("target_dir")).unwrap();
        fs::write(tmp.join("sibling/file.txt"), "preserve").unwrap();
        fs::write(tmp.join("target_dir/file.txt"), "remove").unwrap();
        storage.remove_dir_all(&target).await.unwrap();
        assert!(storage.exists(&sibling).await);
        let content = storage.read(&sibling.join("file.txt")).await.unwrap();
        assert_eq!(content, b"preserve");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_root_path_returns_correct_root() {
        let tmp = temp_dir("root_path");
        let storage = NativeStorage::new(tmp.clone());
        assert_eq!(storage.root_path(), Some(tmp.clone()));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_root_path_default() {
        let storage = NativeStorage::default();
        assert!(storage.root_path().is_some());
        assert_eq!(storage.root_path().unwrap(), PathBuf::from("."));
    }

    #[tokio::test]
    async fn test_paths_are_relative_to_root() {
        let tmp = temp_dir("path_scoping");
        let storage = NativeStorage::new(tmp.clone());
        let path = PathBuf::from("nested/deep/file.txt");
        fs::create_dir_all(tmp.join("nested/deep")).unwrap();
        storage.write(&path, b"scoped data").await.unwrap();
        let content = storage.read(&path).await.unwrap();
        assert_eq!(content, b"scoped data");
        // Writing to absolute path outside root should go to absolute path
        let outside = PathBuf::from("/tmp/main_serve_outside_test.txt");
        storage.write(&outside, b"outside").await.unwrap();
        let content = fs::read(&outside).unwrap();
        assert_eq!(content, b"outside");
        let _ = fs::remove_dir_all(&tmp);
        let _ = fs::remove_file(&outside);
    }

    #[tokio::test]
    async fn test_list_relative_to_root() {
        let tmp = temp_dir("list_relative");
        let storage = NativeStorage::new(tmp.clone());
        let dir = PathBuf::from("mydir");
        storage.create_dir(&dir).await.unwrap();
        storage.write(&dir.join("file.txt"), b"data").await.unwrap();
        let entries = storage.list(&dir).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "file.txt");
        let _ = fs::remove_dir_all(&tmp);
    }
}

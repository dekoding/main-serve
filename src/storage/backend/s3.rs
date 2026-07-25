/// AWS S3 storage backend implementation.
///
/// Implements the `Storage` trait using the AWS SDK for Rust (s3 1.x) to perform
/// file operations against an S3 bucket. Uses virtual directories (prefixes
/// ending with `/`) to represent directory structures, and directory marker
/// objects for empty directories.
use std::path::{Path, PathBuf};

use aws_credential_types::Credentials;
use aws_sdk_s3::Client as S3Client;
use aws_smithy_types::byte_stream::ByteStream;
use aws_types::region::Region;
use bytes::Bytes;
use std::io::Cursor;

use crate::config::types::StoreConfig;
use crate::storage::{DirEntry, FileMetadata, Result, Storage, StorageError};

/// AWS S3 storage backend implementation.
pub struct S3Storage {
    client: S3Client,
    bucket: String,
}

impl S3Storage {
    /// Create a new S3 storage backend from a store configuration.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::InvalidBackend` if the S3 configuration
    /// section is missing from the store config.
    pub async fn new(config: &StoreConfig) -> Result<Self> {
        let s3 = config
            .s3
            .as_ref()
            .ok_or_else(|| {
                StorageError::InvalidBackend(
                    "s3 config section must be present for s3 backend".to_string(),
                )
            })?
            .clone();

        let credentials = Credentials::new(
            s3.access_key.clone(),
            s3.secret_key.clone(),
            None,
            None,
            "static",
        );

        let sdk_config = aws_config::from_env()
            .region(Region::new(s3.region.clone()))
            .credentials_provider(credentials)
            .load()
            .await;

        let client = S3Client::new(&sdk_config);

        Ok(Self {
            client,
            bucket: s3.bucket,
        })
    }

    /// Convert a storage path to an S3 object key.
    fn path_to_key(&self, path: &Path) -> String {
        let path_str = path.to_string_lossy();
        path_str.strip_prefix('/').unwrap_or(&path_str).to_string()
    }

    /// Map an SDK error to a `StorageError`.
    fn map_error<E: std::fmt::Display>(err: E, key: &str) -> StorageError {
        let msg = err.to_string().to_lowercase();
        if msg.contains("no such bucket")
            || msg.contains("no such key")
            || msg.contains("notfound")
            || msg.contains("404")
        {
            StorageError::NotFound(PathBuf::from(key))
        } else if msg.contains("accessdenied")
            || msg.contains("access denied")
            || msg.contains("forbidden")
            || msg.contains("403")
        {
            StorageError::Authentication(format!("S3 access denied: {err}"))
        } else if msg.contains("503") || msg.contains("service unavailable") {
            StorageError::ServiceUnavailable(format!("S3 service unavailable: {err}"))
        } else {
            StorageError::Internal(format!("S3 error: {err}"))
        }
    }

    /// Check if a key exists in the bucket.
    async fn key_exists(&self, key: &str) -> bool {
        self.client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .is_ok()
    }

    /// Collect body from a successful GetObject response.
    async fn collect_body(body: ByteStream) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut stream = body;
        while let Some(chunk) = stream.next().await {
            bytes.extend_from_slice(
                &chunk
                    .map_err(|e| StorageError::Internal(format!("Failed to read S3 body: {e}")))?,
            );
        }
        Ok(bytes)
    }

    /// Convert a DateTime to SystemTime.
    fn dt_to_system_time(dt: &aws_smithy_types::DateTime) -> Option<std::time::SystemTime> {
        // DateTime::as_secs_since_epoch() may not be available in all smithy-types versions.
        // Fall back to parsing the ISO 8601 string representation.
        let iso = dt.to_string();
        chrono::DateTime::parse_from_rfc3339(&iso)
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Utc).into())
    }
}

#[async_trait::async_trait]
impl Storage for S3Storage {
    async fn exists(&self, path: &Path) -> bool {
        let key = self.path_to_key(path);
        self.key_exists(&key).await
    }

    async fn is_file(&self, path: &Path) -> bool {
        let key = self.path_to_key(path);
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(response) => {
                let is_dir_marker =
                    response.content_length().unwrap_or(0) == 0 && key.ends_with('/');
                !is_dir_marker
            }
            Err(_) => false,
        }
    }

    async fn is_dir(&self, path: &Path) -> bool {
        let key = self.path_to_key(path);
        let marker_key = if key.is_empty() {
            String::new()
        } else if key.ends_with('/') {
            key.clone()
        } else {
            format!("{}/", key)
        };

        if self.key_exists(&marker_key).await {
            return true;
        }

        if key.is_empty() {
            let result = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .delimiter('/')
                .send()
                .await;

            return result.is_ok_and(|r| !r.common_prefixes().is_empty());
        }

        let result = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(&marker_key)
            .delimiter('/')
            .send()
            .await;

        result.is_ok_and(|r| !r.common_prefixes().is_empty() || !r.contents().is_empty())
    }

    async fn read(&self, path: &Path) -> Result<Vec<u8>> {
        let key = self.path_to_key(path);
        let response = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| Self::map_error(e, &key))?;

        Self::collect_body(response.body).await
    }

    async fn open(&self, path: &Path) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let key = self.path_to_key(path);
        let response = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| Self::map_error(e, &key))?;

        let bytes = Self::collect_body(response.body).await?;

        Ok(Box::new(Cursor::new(bytes)))
    }

    async fn seek_read(
        &self,
        path: &Path,
        offset: u64,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let key = self.path_to_key(path);
        let range = format!("bytes={offset}-");

        let response = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .range(&range)
            .send()
            .await
            .map_err(|e| Self::map_error(e, &key))?;

        let bytes = Self::collect_body(response.body).await?;

        Ok(Box::new(Cursor::new(bytes)))
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> Result<()> {
        let key = self.path_to_key(path);
        let body: ByteStream = Bytes::from(contents.to_vec()).into();

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(body)
            .send()
            .await
            .map_err(|e| Self::map_error(e, &key))?;

        Ok(())
    }

    async fn append(&self, path: &Path, contents: &[u8]) -> Result<()> {
        let existing = self.read(path).await?;
        let mut combined = existing;
        combined.extend_from_slice(contents);
        self.write(path, &combined).await
    }

    async fn delete(&self, path: &Path) -> Result<()> {
        let key = self.path_to_key(path);
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| Self::map_error(e, &key))?;

        Ok(())
    }

    async fn metadata(&self, path: &Path) -> Result<FileMetadata> {
        let key = self.path_to_key(path);
        let response = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| Self::map_error(e, &key))?;

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());

        let size = response.content_length().unwrap_or(0) as u64;

        let modified = response.last_modified().and_then(Self::dt_to_system_time);

        Ok(FileMetadata {
            name,
            is_file: true,
            size,
            created: None,
            modified,
        })
    }

    async fn list(&self, dir: &Path) -> Result<Vec<DirEntry>> {
        let prefix = self.path_to_key(dir);
        let marker_key = if prefix.is_empty() {
            String::new()
        } else if prefix.ends_with('/') {
            prefix.clone()
        } else {
            format!("{}/", prefix)
        };

        let list_response = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(&marker_key)
            .delimiter('/')
            .send()
            .await
            .map_err(|e| Self::map_error(e, prefix.as_str()))?;

        let mut entries: Vec<DirEntry> = list_response
            .contents()
            .iter()
            .filter_map(|obj| {
                let obj_key = obj.key()?;
                if obj_key == marker_key {
                    return None;
                }
                if !obj_key.starts_with(&marker_key) {
                    return None;
                }
                let relative = obj_key.strip_prefix(&marker_key)?;
                if relative.is_empty() || relative.ends_with('/') {
                    return None;
                }
                let name = match relative.rsplit('/').next() {
                    Some(n) => n.to_string(),
                    None => return None,
                };
                let entry_path = PathBuf::from(dir).join(relative);
                let size = obj.size().unwrap_or(0) as u64;

                Some(DirEntry::new(name, false, size).with_path(entry_path))
            })
            .collect();

        for prefix_item in list_response.common_prefixes() {
            let full_prefix = match prefix_item.prefix() {
                Some(p) => p,
                None => continue,
            };
            let stripped = full_prefix
                .strip_prefix(&marker_key)
                .unwrap_or(full_prefix)
                .strip_suffix('/')
                .unwrap_or(full_prefix);
            let name = stripped.rsplit('/').next().unwrap_or(stripped).to_string();
            let entry_path = PathBuf::from(dir).join(stripped);

            entries.push(DirEntry::new(name, true, 0).with_path(entry_path));
        }

        Ok(entries)
    }

    async fn create_dir(&self, path: &Path) -> Result<()> {
        let key = self.path_to_key(path);
        let dir_key = if key.ends_with('/') {
            key
        } else {
            format!("{}/", key)
        };

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&dir_key)
            .content_type("application/x-directory")
            .body(ByteStream::from(Vec::new()))
            .send()
            .await
            .map_err(|e| Self::map_error(e, &dir_key))?;

        Ok(())
    }

    async fn create_dir_all(&self, path: &Path) -> Result<()> {
        let key = self.path_to_key(path);
        let dir_key = if key.ends_with('/') {
            key.clone()
        } else {
            format!("{}/", key)
        };

        let mut parts: Vec<String> = Vec::new();
        for component in Path::new(&dir_key).components() {
            if let std::path::Component::Normal(c) = component {
                parts.push(c.to_string_lossy().to_string());
            }
        }

        for i in 0..=parts.len() {
            let partial = parts[..i].join("/");
            if !partial.is_empty() {
                let dir_key = format!("{}/", partial);
                let _ = self
                    .client
                    .put_object()
                    .bucket(&self.bucket)
                    .key(&dir_key)
                    .content_type("application/x-directory")
                    .body(ByteStream::from(Vec::new()))
                    .send()
                    .await;
            }
        }

        Ok(())
    }

    async fn remove_dir(&self, path: &Path) -> Result<()> {
        let key = self.path_to_key(path);
        let prefix = if key.ends_with('/') {
            key.clone()
        } else {
            format!("{}/", key)
        };

        let result = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(&prefix)
            .send()
            .await
            .map_err(|e| Self::map_error(e, &prefix))?;

        if !result.contents().is_empty() {
            return Err(StorageError::DirectoryNotEmpty(PathBuf::from(path)));
        }

        let _ = self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(&prefix)
            .send()
            .await;

        Ok(())
    }

    async fn remove_dir_all(&self, path: &Path) -> Result<()> {
        let key = self.path_to_key(path);
        let prefix = if key.ends_with('/') {
            key.clone()
        } else {
            format!("{}/", key)
        };

        let result = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(&prefix)
            .send()
            .await
            .map_err(|e| Self::map_error(e, &prefix))?;

        for obj in result.contents() {
            if let Some(key) = obj.key() {
                let _ = self
                    .client
                    .delete_object()
                    .bucket(&self.bucket)
                    .key(key)
                    .send()
                    .await;
            }
        }

        Ok(())
    }

    async fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        self.copy(from, to).await?;
        self.delete(from).await
    }

    async fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        let from_key = self.path_to_key(from);
        let to_key = self.path_to_key(to);

        let response = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(&from_key)
            .send()
            .await
            .map_err(|e| Self::map_error(e, &from_key))?;

        let content_type = response
            .content_type()
            .map(String::from)
            .unwrap_or_else(|| "application/octet-stream".to_string());

        self.client
            .copy_object()
            .bucket(&self.bucket)
            .copy_source(format!("{}/{}", self.bucket, from_key))
            .key(&to_key)
            .content_type(&content_type)
            .send()
            .await
            .map_err(|e| Self::map_error(e, &to_key))?;

        Ok(())
    }

    async fn canonicalize(&self, path: &Path) -> Result<PathBuf> {
        let key = self.path_to_key(path);
        Ok(PathBuf::from(format!("s3://{}/{}", self.bucket, key)))
    }

    async fn size(&self, path: &Path) -> Result<u64> {
        let key = self.path_to_key(path);
        let response = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| Self::map_error(e, &key))?;

        Ok(response.content_length().unwrap_or(0) as u64)
    }
}

#[cfg(all(test, feature = "s3"))]
/// Integration tests for the S3 storage backend.
mod tests {
    use super::*;

    /// Creates an S3 storage instance using the AWS default credentials.
    fn make_storage() -> S3Storage {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .enable_io()
            .build()
            .unwrap();
        let config = rt.block_on(aws_config::load_from_env());
        S3Storage {
            client: S3Client::new(&config),
            bucket: "test".to_string(),
        }
    }

    #[test]
    /// Tests path-to-key conversion for various input paths.
    fn test_path_to_key() {
        let storage = make_storage();
        assert_eq!(
            storage.path_to_key(Path::new("/foo/bar.txt")),
            "foo/bar.txt"
        );
        assert_eq!(
            storage.path_to_key(Path::new("relative/path.txt")),
            "relative/path.txt"
        );
        assert_eq!(storage.path_to_key(Path::new("/")), "");
        assert_eq!(storage.path_to_key(Path::new("/a/b/c")), "a/b/c");
    }

    #[test]
    /// Tests that 404-style errors map to StorageError::NotFound.
    fn test_map_error_not_found() {
        assert!(matches!(
            S3Storage::map_error("NoSuchKey notfound 404", "test.txt"),
            StorageError::NotFound(_)
        ));
        assert!(matches!(
            S3Storage::map_error("some 404 error", "test.txt"),
            StorageError::NotFound(_)
        ));
        assert!(matches!(
            S3Storage::map_error("notfound in message", "test.txt"),
            StorageError::NotFound(_)
        ));
    }

    #[test]
    /// Tests that 403/access-denied errors map to StorageError::Authentication.
    fn test_map_error_forbidden() {
        assert!(matches!(
            S3Storage::map_error("accessdenied", "test.txt"),
            StorageError::Authentication(_)
        ));
        assert!(matches!(
            S3Storage::map_error("access denied", "test.txt"),
            StorageError::Authentication(_)
        ));
        assert!(matches!(
            S3Storage::map_error("forbidden 403", "test.txt"),
            StorageError::Authentication(_)
        ));
    }

    #[test]
    /// Tests that 503 errors map to StorageError::ServiceUnavailable.
    fn test_map_error_service_unavailable() {
        assert!(matches!(
            S3Storage::map_error("503 service unavailable", "test.txt"),
            StorageError::ServiceUnavailable(_)
        ));
        assert!(matches!(
            S3Storage::map_error("slow down 503", "test.txt"),
            StorageError::ServiceUnavailable(_)
        ));
    }

    #[test]
    /// Tests that directory keys are constructed with a trailing slash.
    fn test_create_dir_key_format() {
        let storage = make_storage();
        let key = storage.path_to_key(Path::new("mydir"));
        let dir_key = if key.ends_with('/') {
            key
        } else {
            format!("{}/", key)
        };
        assert_eq!(dir_key, "mydir/");
        let key2 = storage.path_to_key(Path::new("/a/b/c"));
        let dir_key2 = if key2.ends_with('/') {
            key2
        } else {
            format!("{}/", key2)
        };
        assert_eq!(dir_key2, "a/b/c/");
    }
}

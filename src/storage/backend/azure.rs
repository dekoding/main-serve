use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use base64::Engine;
use chrono::{DateTime, FixedOffset, Utc};
use http::Method;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_encode};
use sha2::{Digest, Sha256};

use crate::config::types::StoreConfig;
use crate::storage::{DirEntry, FileMetadata, Result, Storage, StorageError};

const AZURE_BLOB_API_VERSION: &str = "2023-11-03";

/// Maximum number of retry attempts for copy status polling.
const MAX_COPY_RETRIES: u32 = 60;

/// Characters that must be percent-encoded in Azure blob names.
const BLOB_NAME_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'/')
    .remove(b'~')
    .remove(b':');

/// Azure Blob Storage backend implementation.
///
/// Uses the Azure Blob Storage REST API with Shared Key authentication
/// via `reqwest` for HTTP requests. Directories are virtual (blob name
/// prefixes ending with `/`).
pub struct AzureStorage {
    /// Azure storage account name.
    account_name: String,
    /// Azure storage account key (HMAC secret).
    account_key: String,
    /// Azure container name.
    container: String,
    /// HTTP client for API requests.
    client: reqwest::Client,
}

impl AzureStorage {
    /// Create a new Azure storage backend from a store configuration.
    pub fn new(config: &StoreConfig) -> Result<Self> {
        let azure = config
            .azure
            .as_ref()
            .ok_or_else(|| {
                StorageError::InvalidBackend(
                    "azure config section must be present for azure backend".to_string(),
                )
            })?
            .clone();

        Ok(Self {
            account_name: azure.account_name,
            account_key: azure.account_key,
            container: azure.container,
            client: reqwest::Client::new(),
        })
    }

    /// Build the base URL for this container.
    fn base_url(&self) -> String {
        format!(
            "https://{}.blob.core.windows.net/{}",
            self.account_name, self.container
        )
    }

    /// Build a blob URL for the given blob name.
    fn blob_url(&self, blob_name: &str, query: &str) -> String {
        let encoded = percent_encode(blob_name.as_bytes(), BLOB_NAME_ENCODE_SET).to_string();
        if query.is_empty() {
            format!("{}/{}", self.base_url(), encoded)
        } else {
            format!("{}/{}?{}", self.base_url(), encoded, query)
        }
    }

    /// Convert a storage path to a blob name.
    ///
    /// Strips the leading `/` if present. Azure blob names must not
    /// start with `/`.
    fn path_to_blob_name(&self, path: &Path) -> String {
        let path_str = path.to_string_lossy();
        path_str.strip_prefix('/').unwrap_or(&path_str).to_string()
    }

    /// Generate an RFC 3339 timestamp for Azure headers.
    fn rfc3339_timestamp(&self) -> String {
        Utc::now().to_rfc3339()
    }

    /// Build the canonicalized headers string for signing.
    ///
    /// Headers are lowercased, sorted alphabetically, and joined with `\n`.
    fn build_canonicalized_headers(&self) -> String {
        let mut headers: HashMap<String, String> = HashMap::new();
        headers.insert(
            "x-ms-client-request-id".to_string(),
            uuid::Uuid::new_v4().to_string(),
        );
        headers.insert("x-ms-date".to_string(), self.rfc3339_timestamp());
        headers.insert(
            "x-ms-version".to_string(),
            AZURE_BLOB_API_VERSION.to_string(),
        );

        let mut header_lines: Vec<String> = headers
            .iter()
            .map(|(k, v)| format!("{}:{}\n", k.to_lowercase(), v))
            .collect();
        header_lines.sort();
        header_lines.join("")
    }

    /// Build the canonicalized resource string for signing.
    fn build_canonicalized_resource(&self, blob_name: &str, query: &str) -> String {
        let mut resource = format!("/{}/{}/{}", self.account_name, self.container, blob_name);
        if !query.is_empty() {
            resource.push('\n');
            resource.push_str(query);
        }
        resource
    }

    /// Compute HMAC-SHA256 manually to avoid hmac crate digest version conflicts.
    fn compute_hmac_sha256(key: &[u8], message: &[u8]) -> Vec<u8> {
        let block_size = 64;

        let mut k = vec![0u8; block_size];
        if key.len() > block_size {
            let mut hasher = Sha256::new();
            hasher.update(key);
            k = hasher.finalize().to_vec();
        } else {
            k[..key.len()].copy_from_slice(key);
        }

        let mut ipad = vec![0x36u8; block_size];
        let mut opad = vec![0x5cu8; block_size];
        for (i, (i_op, o_op)) in ipad.iter_mut().zip(opad.iter_mut()).enumerate() {
            if i < key.len() {
                *i_op ^= k[i];
                *o_op ^= k[i];
            }
        }

        let mut inner_hasher = Sha256::new();
        inner_hasher.update(&ipad);
        inner_hasher.update(message);
        let inner_hash = inner_hasher.finalize();

        let mut outer_hasher = Sha256::new();
        outer_hasher.update(opad);
        outer_hasher.update(inner_hash);
        outer_hasher.finalize().to_vec()
    }

    /// Sign a request using Azure Shared Key authentication.
    ///
    /// Returns the `Authorization` header value.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Authentication` if the account key is invalid base64.
    fn sign_request(
        &self,
        verb: &str,
        content_length: u64,
        canonicalized_headers: &str,
        canonicalized_resource: &str,
    ) -> Result<String> {
        let signing_string = format!(
            "{verb}\n\n\n{content_length}\n\n\n\n\n\n\n\n\n\n{canonicalized_headers}\n{canonicalized_resource}\n",
        );

        let key = decode_base64(&self.account_key)?;
        let signature = Self::compute_hmac_sha256(&key, signing_string.as_bytes());

        Ok(format!(
            "SharedKey {}:{}",
            self.account_name,
            base64_encode(&signature)
        ))
    }

    /// Make an authenticated HTTP request and return the response.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Authentication` if the account key is invalid.
    /// Returns `StorageError::ServiceUnavailable` if the HTTP request fails.
    async fn request(
        &self,
        method: Method,
        blob_name: &str,
        query: &str,
        body: Option<Vec<u8>>,
    ) -> Result<reqwest::Response> {
        let url = self.blob_url(blob_name, query);
        let canonicalized_headers = self.build_canonicalized_headers();
        let content_length = body.as_ref().map(|b| b.len() as u64).unwrap_or(0);
        let auth = self.sign_request(
            method.as_str(),
            content_length,
            &canonicalized_headers,
            &self.build_canonicalized_resource(blob_name, query),
        )?;

        let mut request = self.client.request(method, &url);
        request = request.header("Authorization", auth);
        request = request.header("x-ms-blob-type", "BlockBlob");
        for (k, v) in Self::parse_canonicalized_headers(&canonicalized_headers) {
            request = request.header(k, v);
        }

        if let Some(data) = body {
            request = request.body(data);
        }

        let response = request
            .send()
            .await
            .map_err(|e| StorageError::ServiceUnavailable(format!("Azure request failed: {e}")))?;

        Ok(response)
    }

    /// Process an HTTP response, mapping status codes to errors.
    async fn handle_response(
        &self,
        response: reqwest::Response,
        blob_name: &str,
    ) -> Result<reqwest::Response> {
        let status = response.status();
        match status {
            reqwest::StatusCode::OK
            | reqwest::StatusCode::CREATED
            | reqwest::StatusCode::NO_CONTENT
            | reqwest::StatusCode::ACCEPTED
            | reqwest::StatusCode::CONFLICT => Ok(response),
            reqwest::StatusCode::NOT_FOUND => Err(StorageError::NotFound(PathBuf::from(blob_name))),
            reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::UNAUTHORIZED => Err(
                StorageError::Authentication("Azure authentication failed".to_string()),
            ),
            reqwest::StatusCode::TOO_MANY_REQUESTS => Err(StorageError::ServiceUnavailable(
                "Azure rate limited".to_string(),
            )),
            s if s.is_server_error() => Err(StorageError::ServiceUnavailable(format!(
                "Azure service error: {status}"
            ))),
            _ => Ok(response),
        }
    }

    /// Parse canonicalized headers into key-value pairs.
    fn parse_canonicalized_headers(s: &str) -> Vec<(String, String)> {
        s.lines()
            .filter(|l| !l.is_empty())
            .filter_map(|line| {
                let colon_pos = line.find(':')?;
                Some((
                    line[..colon_pos].to_string(),
                    line[colon_pos + 1..].to_string(),
                ))
            })
            .collect()
    }

    /// Parse the XML response from a container list operation.
    ///
    /// Returns a tuple of (files with sizes, directory names).
    fn parse_list_xml(&self, xml: &str, prefix: &str) -> Result<ListResult> {
        let doc: ListResponse = quick_xml::de::from_str(xml).map_err(|e| {
            StorageError::Internal(format!("Failed to parse Azure list response: {e}"))
        })?;

        let mut files = Vec::new();
        let mut dirs = Vec::new();

        let prefix = prefix.trim_end_matches('/');

        if let Some(ref blob_group) = doc.blob_group {
            for blob in &blob_group.blobs {
                if blob.name == prefix || !blob.name.starts_with(prefix) {
                    continue;
                }
                let suffix = &blob.name[prefix.len()..];
                let suffix = suffix.strip_prefix('/').unwrap_or(suffix);
                if let Some(slash_pos) = suffix.find('/') {
                    let dir_name = &suffix[..slash_pos];
                    if !dirs.contains(&dir_name.to_string()) {
                        dirs.push(dir_name.to_string());
                    }
                } else if !suffix.is_empty() {
                    let size = blob
                        .properties
                        .size
                        .as_ref()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    files.push((suffix.to_string(), size));
                }
            }
        }

        for prefix_elem in &doc.prefixes {
            if !prefix_elem.name.starts_with(prefix) {
                continue;
            }
            let suffix = &prefix_elem.name[prefix.len()..];
            let dir_name = suffix.trim_end_matches('/');
            if !dir_name.is_empty() && !dirs.contains(&dir_name.to_string()) {
                dirs.push(dir_name.to_string());
            }
        }

        Ok((files, dirs))
    }
}

/// Type alias for the return value of `parse_list_xml`.
type ListResult = (Vec<(String, u64)>, Vec<String>);

/// Azure list containers response XML structure.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename = "EnumerationResults", default)]
struct ListResponse {
    #[serde(rename = "Blobs", default)]
    blob_group: Option<BlobGroup>,
    #[serde(rename = "Prefixes", default)]
    prefixes: Vec<BlobPrefix>,
}

/// Group of blobs in the list response.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename = "Blobs", default)]
struct BlobGroup {
    #[serde(rename = "Blob", default)]
    blobs: Vec<AzureBlob>,
}

/// A single blob entry in the list response.
#[derive(Debug, Clone, serde::Deserialize)]
struct AzureBlob {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Properties", default)]
    properties: BlobProperties,
}

/// Properties of a blob from the list response.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct BlobProperties {
    #[serde(rename = "Content-Length", default)]
    size: Option<String>,
}

/// A virtual directory prefix from the list response.
#[derive(Debug, Clone, serde::Deserialize)]
struct BlobPrefix {
    #[serde(rename = "Name")]
    name: String,
}

/// Decode a base64-encoded string to bytes.
///
/// # Errors
///
/// Returns `StorageError::Authentication` if the input is not valid base64.
fn decode_base64(s: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|_| StorageError::Authentication("Invalid base64 in account key".to_string()))
}

/// Encode bytes as base64.
fn base64_encode(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[async_trait::async_trait]
impl Storage for AzureStorage {
    async fn exists(&self, path: &Path) -> bool {
        let blob_name = self.path_to_blob_name(path);
        let response = match self
            .request(Method::HEAD, &blob_name, "comp=properties", None)
            .await
        {
            Ok(r) => r,
            Err(_) => return false,
        };
        self.handle_response(response, &blob_name).await.is_ok()
    }

    async fn is_file(&self, path: &Path) -> bool {
        self.exists(path).await
    }

    async fn is_dir(&self, path: &Path) -> bool {
        let blob_name = self.path_to_blob_name(path);
        let prefix = if blob_name.is_empty() {
            String::new()
        } else {
            format!("{blob_name}/")
        };
        let query = format!("restype=container&comp=list&delimiter=/&prefix={prefix}");
        let response = match self.request(Method::GET, "", &query, None).await {
            Ok(r) => r,
            Err(_) => return false,
        };
        let response = match self.handle_response(response, &blob_name).await {
            Ok(r) => r,
            Err(_) => return false,
        };

        let body = match response.text().await {
            Ok(b) => b,
            Err(_) => return false,
        };

        let (files, dirs) = match self.parse_list_xml(&body, &prefix) {
            Ok((f, d)) => (f, d),
            Err(_) => return false,
        };

        !files.is_empty() || !dirs.is_empty()
    }

    async fn read(&self, path: &Path) -> Result<Vec<u8>> {
        let blob_name = self.path_to_blob_name(path);
        let response = self
            .request(Method::GET, &blob_name, "comp=properties", None)
            .await?;
        let response = self.handle_response(response, &blob_name).await?;
        let bytes = response
            .bytes()
            .await
            .map_err(|e| StorageError::Internal(format!("Failed to read response body: {e}")))?;
        Ok(bytes.to_vec())
    }

    async fn open(&self, path: &Path) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let blob_name = self.path_to_blob_name(path);
        let response = self
            .request(Method::GET, &blob_name, "comp=properties", None)
            .await?;
        let response = self.handle_response(response, &blob_name).await?;

        let bytes = response
            .bytes()
            .await
            .map_err(|e| StorageError::Internal(format!("Failed to read response body: {e}")))?;

        Ok(Box::new(Cursor::new(bytes)))
    }

    async fn seek_read(
        &self,
        path: &Path,
        offset: u64,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let blob_name = self.path_to_blob_name(path);

        let url = self.blob_url(&blob_name, "comp=properties");
        let canonicalized_headers = self.build_canonicalized_headers();
        let auth = self.sign_request(
            "GET",
            0,
            &canonicalized_headers,
            &self.build_canonicalized_resource(&blob_name, "comp=properties"),
        )?;

        let mut request = self.client.request(Method::GET, &url);
        request = request.header("Authorization", auth);
        request = request.header("x-ms-blob-type", "BlockBlob");
        request = request.header("Range", format!("bytes={offset}-"));
        for (k, v) in Self::parse_canonicalized_headers(&canonicalized_headers) {
            request = request.header(k, v);
        }

        let response = request
            .send()
            .await
            .map_err(|e| StorageError::ServiceUnavailable(format!("Azure request failed: {e}")))?;

        let bytes = response
            .bytes()
            .await
            .map_err(|e| StorageError::Internal(format!("Failed to read response body: {e}")))?;

        Ok(Box::new(Cursor::new(bytes)))
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> Result<()> {
        let blob_name = self.path_to_blob_name(path);
        let response = self
            .request(
                Method::PUT,
                &blob_name,
                "comp=blockblob",
                Some(contents.to_vec()),
            )
            .await?;
        self.handle_response(response, &blob_name).await?;
        Ok(())
    }

    async fn append(&self, path: &Path, contents: &[u8]) -> Result<()> {
        let blob_name = self.path_to_blob_name(path);

        let head_response = match self
            .request(Method::HEAD, &blob_name, "comp=properties", None)
            .await
        {
            Ok(r) => r,
            Err(_) => {
                // If the HEAD request fails, treat as new file
                return self.write(path, contents).await;
            }
        };
        let head_status = head_response.status();

        let mut existing = Vec::new();
        if head_status.is_success() {
            let get_response = self
                .request(Method::GET, &blob_name, "comp=properties", None)
                .await?;
            let get_resp = self.handle_response(get_response, &blob_name).await?;
            existing = get_resp
                .bytes()
                .await
                .map_err(|e| StorageError::Internal(format!("Failed to read existing blob: {e}")))?
                .to_vec();
        }

        let mut merged = existing;
        merged.extend_from_slice(contents);

        self.write(path, &merged).await
    }

    async fn delete(&self, path: &Path) -> Result<()> {
        let blob_name = self.path_to_blob_name(path);
        let response = self.request(Method::DELETE, &blob_name, "", None).await?;
        self.handle_response(response, &blob_name).await?;
        Ok(())
    }

    async fn metadata(&self, path: &Path) -> Result<FileMetadata> {
        let blob_name = self.path_to_blob_name(path);
        let response = self
            .request(Method::HEAD, &blob_name, "comp=properties", None)
            .await?;
        let response = self.handle_response(response, &blob_name).await?;

        let headers = response.headers();

        let size_str = headers
            .get("x-ms-blob-content-length")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("0");
        let size: u64 = size_str.parse().unwrap_or(0);

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());

        let modified_str = headers
            .get("x-ms-last-modified")
            .and_then(|v| v.to_str().ok());
        let modified: Option<SystemTime> = modified_str.and_then(|s| {
            DateTime::<FixedOffset>::parse_from_rfc2822(s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc).into())
        });

        Ok(FileMetadata {
            name,
            is_file: true,
            size,
            created: None,
            modified,
        })
    }

    async fn list(&self, dir: &Path) -> Result<Vec<DirEntry>> {
        let blob_name = self.path_to_blob_name(dir);
        let prefix = if blob_name.is_empty() || blob_name == "/" {
            String::new()
        } else {
            format!("{blob_name}/")
        };

        let query = format!("restype=container&comp=list&delimiter=/&prefix={prefix}");
        let response = self.request(Method::GET, "", &query, None).await?;
        let response = self.handle_response(response, "").await?;

        let body = response.text().await.map_err(|e| {
            StorageError::Internal(format!("Failed to read list response body: {e}"))
        })?;

        let (files, dirs) = self.parse_list_xml(&body, &prefix)?;

        let mut entries = Vec::new();

        for (file_name, size) in files {
            let file_path = PathBuf::from(format!("{prefix}{file_name}"));
            entries.push(DirEntry::new(file_name.clone(), false, size).with_path(file_path));
        }

        for dir_name in dirs {
            let dir_path = PathBuf::from(format!("{prefix}{dir_name}"));
            entries.push(DirEntry::new(dir_name.clone(), true, 0).with_path(dir_path));
        }

        Ok(entries)
    }

    async fn create_dir(&self, path: &Path) -> Result<()> {
        let mut blob_name = self.path_to_blob_name(path);
        if !blob_name.is_empty() && !blob_name.ends_with('/') {
            blob_name.push('/');
        }
        self.write(Path::new(&blob_name), b"").await
    }

    async fn create_dir_all(&self, path: &Path) -> Result<()> {
        let blob_name = self.path_to_blob_name(path);
        let prefix = if blob_name.is_empty() {
            String::new()
        } else {
            format!("{blob_name}/")
        };

        let query = format!("restype=container&comp=list&delimiter=/&prefix={prefix}");
        let response = match self.request(Method::GET, "", &query, None).await {
            Ok(r) => r,
            Err(_) => return self.create_dir(path).await,
        };

        if response.status().is_success() {
            let body = response.text().await.map_err(|e| {
                StorageError::Internal(format!("Failed to read list response: {e}"))
            })?;

            let (files, dirs) = self.parse_list_xml(&body, &prefix)?;
            if !files.is_empty() || !dirs.is_empty() {
                return Ok(());
            }
        }

        self.create_dir(path).await
    }

    async fn remove_dir(&self, path: &Path) -> Result<()> {
        let blob_name = self.path_to_blob_name(path);
        let prefix = if blob_name.is_empty() {
            String::new()
        } else {
            format!("{blob_name}/")
        };

        let query = format!("restype=container&comp=list&delimiter=/&prefix={prefix}");
        let response = self.request(Method::GET, "", &query, None).await?;
        let response = self.handle_response(response, "").await?;

        if response.status().is_success() {
            let body = response.text().await.map_err(|e| {
                StorageError::Internal(format!("Failed to read list response: {e}"))
            })?;

            let (files, _) = self.parse_list_xml(&body, &prefix)?;
            for (file, _size) in files {
                let file_path = PathBuf::from(format!("{prefix}{file}"));
                self.delete(&file_path).await?;
            }
        }

        let dir_path = PathBuf::from(&format!("{blob_name}/"));
        self.delete(&dir_path).await?;

        Ok(())
    }

    async fn remove_dir_all(&self, path: &Path) -> Result<()> {
        let blob_name = self.path_to_blob_name(path);
        let prefix = if blob_name.is_empty() {
            String::new()
        } else {
            format!("{blob_name}/")
        };

        loop {
            let query = format!("restype=container&comp=list&delimiter=/&prefix={prefix}");
            let response = match self.request(Method::GET, "", &query, None).await {
                Ok(r) => r,
                Err(_) => break,
            };

            if !response.status().is_success() {
                break;
            }

            let body = response.text().await.map_err(|e| {
                StorageError::Internal(format!("Failed to read list response: {e}"))
            })?;

            let (files, sub_dirs) = self.parse_list_xml(&body, &prefix)?;

            for (file, _size) in files {
                let file_path = PathBuf::from(format!("{prefix}{file}"));
                self.delete(&file_path).await?;
            }

            if sub_dirs.is_empty() {
                break;
            }

            let new_prefixes: Vec<String> =
                sub_dirs.iter().map(|d| format!("{prefix}{d}/")).collect();

            for new_prefix in new_prefixes {
                let sub_query =
                    format!("restype=container&comp=list&delimiter=/&prefix={new_prefix}");
                let sub_response = match self.request(Method::GET, "", &sub_query, None).await {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                if sub_response.status().is_success() {
                    let sub_body = sub_response
                        .text()
                        .await
                        .map_err(|e| StorageError::Internal(format!("Failed to read list: {e}")))?;
                    let (sub_files, _) = self.parse_list_xml(&sub_body, &new_prefix)?;
                    for (f, _size) in sub_files {
                        let f_path = PathBuf::from(format!("{new_prefix}{f}"));
                        self.delete(&f_path).await?;
                    }
                }
            }
        }

        Ok(())
    }

    async fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let to_blob = self.path_to_blob_name(to);

        let response = self
            .request(Method::PUT, &to_blob, "comp=copy", None)
            .await?;
        let copy_status: String = response
            .headers()
            .get("x-ms-copy-status")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        self.handle_response(response, &to_blob).await?;

        if copy_status == "pending" || copy_status == "success" {
            for _ in 0..60 {
                let check_url = self.blob_url(&to_blob, "comp=properties");
                let check_response = self.client.head(&check_url).send().await;
                if let Ok(r) = check_response {
                    let status = r
                        .headers()
                        .get("x-ms-copy-status")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    if status == "success" {
                        break;
                    }
                    if status == "failure" {
                        return Err(StorageError::Internal(
                            "Copy failed during rename".to_string(),
                        ));
                    }
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }

        self.delete(from).await?;
        Ok(())
    }

    async fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        let _from = from;
        let to_blob = self.path_to_blob_name(to);

        let response = self
            .request(Method::PUT, &to_blob, "comp=copy", None)
            .await?;
        let copy_status: String = response
            .headers()
            .get("x-ms-copy-status")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        self.handle_response(response, &to_blob).await?;

        if copy_status == "pending" || copy_status == "success" {
            for _ in 0..MAX_COPY_RETRIES {
                let check_url = self.blob_url(&to_blob, "comp=properties");
                let check_response = self.client.head(&check_url).send().await;
                if let Ok(r) = check_response {
                    let status = r
                        .headers()
                        .get("x-ms-copy-status")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    if status == "success" || status == "invalid" {
                        break;
                    }
                    if status == "failure" {
                        return Err(StorageError::Internal("Copy failed".to_string()));
                    }
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }

        Ok(())
    }

    async fn canonicalize(&self, path: &Path) -> Result<PathBuf> {
        let blob_name = self.path_to_blob_name(path);
        let url = format!(
            "{}/{}",
            self.base_url(),
            percent_encode(blob_name.as_bytes(), BLOB_NAME_ENCODE_SET)
        );
        Ok(PathBuf::from(url))
    }

    async fn size(&self, path: &Path) -> Result<u64> {
        let blob_name = self.path_to_blob_name(path);
        let response = self
            .request(Method::HEAD, &blob_name, "comp=properties", None)
            .await?;
        let response = self.handle_response(response, &blob_name).await?;

        let headers = response.headers();
        let size_str = headers
            .get("x-ms-blob-content-length")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("0");
        let size: u64 = size_str
            .parse()
            .map_err(|e| StorageError::Internal(format!("Failed to parse blob size: {e}")))?;
        Ok(size)
    }
}

#[cfg(all(test, feature = "azure"))]
mod tests {
    use super::*;

    #[test]
    fn test_path_to_blob_name() {
        let storage = AzureStorage {
            account_name: "test".to_string(),
            account_key: "test".to_string(),
            container: "test".to_string(),
            client: reqwest::Client::new(),
        };

        assert_eq!(
            storage.path_to_blob_name(Path::new("/foo/bar.txt")),
            "foo/bar.txt"
        );
        assert_eq!(
            storage.path_to_blob_name(Path::new("relative/path.txt")),
            "relative/path.txt"
        );
        assert_eq!(storage.path_to_blob_name(Path::new("/")), "");
        assert_eq!(storage.path_to_blob_name(Path::new("/a/b/c")), "a/b/c");
    }

    #[test]
    fn test_build_canonicalized_resource() {
        let storage = AzureStorage {
            account_name: "myaccount".to_string(),
            account_key: "test".to_string(),
            container: "mycontainer".to_string(),
            client: reqwest::Client::new(),
        };

        let resource = storage.build_canonicalized_resource("path/to/file.txt", "");
        assert_eq!(resource, "/myaccount/mycontainer/path/to/file.txt");

        let resource2 = storage.build_canonicalized_resource("file.txt", "comp=list");
        assert_eq!(resource2, "/myaccount/mycontainer/file.txt\ncomp=list");
    }

    #[test]
    fn test_sign_request_format() {
        let storage = AzureStorage {
            account_name: "myaccount".to_string(),
            account_key: "dGVzdA==".to_string(),
            container: "mycontainer".to_string(),
            client: reqwest::Client::new(),
        };

        let headers = storage.build_canonicalized_headers();
        let resource = storage.build_canonicalized_resource("file.txt", "");

        let result = storage.sign_request("GET", 0, &headers, &resource);
        assert!(result.is_ok());

        let auth = result.unwrap();
        assert!(auth.starts_with("SharedKey myaccount:"));
    }
}

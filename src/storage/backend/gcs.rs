/// Google Cloud Storage backend implementation.
///
/// Provides a full implementation of the `Storage` trait using the GCS REST API
/// with `OAuth2` service account authentication (RS256 JWT bearer token flow).
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use tokio_util::io::StreamReader;

use crate::config::types::StoreConfig;
use crate::storage::{DirEntry, FileMetadata, Result, Storage, StorageError};

/// GCS JSON API base URL.
const GCS_API_URL: &str = "https://storage.googleapis.com/storage/v1";

/// Google `OAuth2` token endpoint.
const OAUTH2_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// Safety margin subtracted from token expiry to avoid using near-expired tokens.
const TOKEN_SAFETY_MARGIN_SECS: u64 = 60;

/// Timeout for HTTP client requests to GCS and `OAuth2` endpoints.
const HTTP_TIMEOUT_SECS: u64 = 30;

/// Service account credentials parsed from JSON.
#[derive(Clone, Deserialize)]
/// Parsed GCS service account credentials for `OAuth2` JWT bearer authentication.
struct ServiceAccountCredentials {
    /// The client email address.
    client_email: String,
    /// The RSA private key in PKCS#8 PEM format.
    private_key: String,
}

impl std::fmt::Debug for ServiceAccountCredentials {
    /// Formats the struct with the `private_key` field redacted.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceAccountCredentials")
            .field("client_email", &self.client_email)
            .field("private_key", &"[REDACTED]")
            .finish()
    }
}

/// Cached `OAuth2` access token.
#[derive(Clone)]
/// An `OAuth2` access token with an associated expiry timestamp.
struct AuthToken {
    /// The access token string.
    token: String,
    /// Unix timestamp when the token expires.
    expires_at: u64,
}

impl AuthToken {
    /// Check if this token is still valid (with safety margin).
    fn is_valid(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now < self.expires_at - TOKEN_SAFETY_MARGIN_SECS
    }
}

impl std::fmt::Debug for AuthToken {
    /// Formats the struct with the token field hidden (non-exhaustive).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthToken")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Token cache with lazy refresh logic.
struct AuthTokenCache {
    /// The current cached token, if any.
    /// Uses `tokio::sync::Mutex` because `MutexGuard` is `Send` in async contexts,
    /// allowing token refresh to hold the lock across an `.await` point.
    token: tokio::sync::Mutex<Option<AuthToken>>,
    /// Credentials used to obtain tokens.
    credentials: ServiceAccountCredentials,
    /// reqwest client for `OAuth2` token requests.
    client: reqwest::Client,
}

impl AuthTokenCache {
    /// Initializes the cache with the provided credentials and an HTTP client.
    fn new(credentials: ServiceAccountCredentials) -> std::result::Result<Self, StorageError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| StorageError::Internal(format!("Failed to build HTTP client: {e}")))?;

        Ok(Self {
            token: tokio::sync::Mutex::new(None),
            credentials,
            client,
        })
    }

    /// Get a valid access token, refreshing if necessary.
    ///
    /// Uses double-checked locking: checks the cache with a lock,
    /// then refreshes with a write lock if needed.
    async fn get_token(&self) -> std::result::Result<String, StorageError> {
        {
            let guard = self.token.lock().await;
            if let Some(ref token) = *guard
                && token.is_valid()
            {
                return Ok(token.token.clone());
            }
        }

        let guard = self.token.lock().await;

        // Double-check: another caller may have refreshed while we waited for the lock.
        if let Some(ref token) = *guard
            && token.is_valid()
        {
            return Ok(token.token.clone());
        }

        let new_token = {
            drop(guard);
            self.fetch_token().await?
        };
        *self.token.lock().await = Some(new_token.clone());
        Ok(new_token.token)
    }

    async fn fetch_token(&self) -> std::result::Result<AuthToken, StorageError> {
        #[derive(Deserialize)]
        /// JSON response from the GCS `OAuth2` token exchange endpoint.
        struct TokenResponse {
            access_token: String,
            expires_in: u64,
        }

        let jwt = self.create_jwt()?;

        let resp = self
            .client
            .post(OAUTH2_TOKEN_URL)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &jwt),
            ])
            .send()
            .await
            .map_err(|e| StorageError::Authentication(format!("Failed to exchange JWT: {e}")))?;

        let status = resp.status();
        let body = resp.text().await.map_err(|e| {
            StorageError::Authentication(format!("Failed to read token response: {e}"))
        })?;

        if !status.is_success() {
            return Err(StorageError::Authentication(format!(
                "OAuth2 token exchange failed ({status}): {body}"
            )));
        }

        let token_resp: TokenResponse = serde_json::from_str(&body).map_err(|e| {
            StorageError::Authentication(format!("Failed to parse token response: {e}"))
        })?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Ok(AuthToken {
            token: token_resp.access_token,
            expires_at: now + token_resp.expires_in - TOKEN_SAFETY_MARGIN_SECS,
        })
    }

    fn create_jwt(&self) -> std::result::Result<String, StorageError> {
        use jsonwebtoken::Header;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .cast_signed();

        let claims = GcsJwtClaims {
            iss: self.credentials.client_email.clone(),
            sub: self.credentials.client_email.clone(),
            scope: "https://www.googleapis.com/auth/cloud-platform".to_string(),
            aud: OAUTH2_TOKEN_URL.to_string(),
            exp: now + 3600,
            iat: now,
        };

        let signing_key =
            jsonwebtoken::EncodingKey::from_rsa_pem(self.credentials.private_key.as_bytes())
                .map_err(|e| {
                    StorageError::Authentication(format!("Failed to load RSA key: {e}"))
                })?;

        let header = Header::new(jsonwebtoken::Algorithm::RS256);
        jsonwebtoken::encode(&header, &claims, &signing_key)
            .map_err(|e| StorageError::Authentication(format!("Failed to create JWT: {e}")))
    }
}

/// JWT claims for GCS `OAuth2` service account authentication.
#[derive(Debug, serde::Serialize)]
/// GCS `OAuth2` JWT claims for service account authentication.
struct GcsJwtClaims {
    iss: String,
    sub: String,
    scope: String,
    aud: String,
    exp: i64,
    iat: i64,
}

/// Google Cloud Storage backend implementation.
pub struct GcsStorage {
    /// GCS bucket name.
    bucket: String,
    /// reqwest client configured for GCS API calls.
    client: reqwest::Client,
    /// `OAuth2` token cache.
    auth: AuthTokenCache,
}

#[derive(Debug, Deserialize)]
/// GCS JSON API object metadata response.
struct GcsObjectMetadata {
    #[serde(rename = "name")]
    name: String,

    #[serde(rename = "size")]
    size: String,

    #[serde(rename = "timeCreated", default)]
    time_created: Option<String>,

    #[serde(rename = "updated", default)]
    updated: Option<String>,
}

impl GcsObjectMetadata {
    /// Parse the size string to a u64.
    fn parse_size(&self) -> u64 {
        self.size.parse().unwrap_or(0)
    }

    /// Parse an RFC 3339 timestamp to `SystemTime`.
    fn parse_timestamp(ts: Option<&String>) -> Option<std::time::SystemTime> {
        ts.and_then(|t| {
            chrono::DateTime::parse_from_rfc3339(t)
                .ok()
                .map(|dt| dt.with_timezone(&chrono::Utc).into())
        })
    }
}

#[derive(Debug, Deserialize)]
/// GCS JSON API list response containing objects and common prefixes.
struct GcsListResponse {
    /// Object items in the listing.
    #[serde(default)]
    items: Option<Vec<GcsObjectMetadata>>,

    /// Common prefixes (virtual directories).
    #[serde(default)]
    prefixes: Option<Vec<String>>,
}

impl GcsStorage {
    /// Create a new GCS storage backend from a store configuration.
    ///
    /// Authenticates immediately with GCS to verify credentials are valid.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::InvalidBackend` if the `gcs` config section is missing.
    /// Returns `StorageError::InvalidBackend` if credentials are empty.
    /// Returns `StorageError::Authentication` if credentials JSON is invalid or the
    /// credentials file cannot be read. Returns `StorageError::Internal` if the
    /// HTTP client fails to build.
    pub fn new(config: &StoreConfig) -> Result<Self> {
        let gcs = config
            .gcs
            .as_ref()
            .ok_or_else(|| {
                StorageError::InvalidBackend(
                    "gcs config section must be present for gcs backend".to_string(),
                )
            })?
            .clone();

        let credentials = Self::load_credentials(&gcs.credentials)?;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| StorageError::Internal(format!("Failed to build HTTP client: {e}")))?;

        let auth = AuthTokenCache::new(credentials)?;

        Ok(Self {
            bucket: gcs.bucket,
            client,
            auth,
        })
    }

    /// Load service account credentials from a JSON string or file path.
    fn load_credentials(raw: &str) -> Result<ServiceAccountCredentials> {
        if raw.is_empty() {
            return Err(StorageError::InvalidBackend(
                "gcs credentials must not be empty".to_string(),
            ));
        }

        // Try parsing as JSON first (raw service account key JSON string).
        if raw.starts_with('{') || raw.starts_with('[') {
            let creds: ServiceAccountCredentials = serde_json::from_str(raw).map_err(|e| {
                StorageError::Authentication(format!("Invalid GCS credentials JSON: {e}"))
            })?;
            return Ok(creds);
        }

        // Treat as file path.
        let contents = std::fs::read_to_string(raw).map_err(|e| {
            StorageError::Authentication(format!("Failed to read credentials file: {e}"))
        })?;

        let creds: ServiceAccountCredentials = serde_json::from_str(&contents).map_err(|e| {
            StorageError::Authentication(format!("Invalid GCS credentials JSON: {e}"))
        })?;

        Ok(creds)
    }

    /// Convert a storage path to a GCS object name (no leading slash).
    fn path_to_object_name(path: &Path) -> String {
        let path_str = path.to_string_lossy();
        let name = path_str.strip_prefix('/').unwrap_or(&path_str);
        name.to_string()
    }

    /// Build a GCS API request builder for an object path.
    fn gcs_request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{GCS_API_URL}{path}");
        self.client.request(method, url)
    }

    /// Map a GCS HTTP response status to a `StorageError`, if applicable.
    fn map_http_error(status: reqwest::StatusCode, object_name: &str, body: &str) -> StorageError {
        match status {
            reqwest::StatusCode::NOT_FOUND => StorageError::NotFound(PathBuf::from(object_name)),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                StorageError::Authentication(format!("GCS returned {status}: {body}"))
            }
            s if s == reqwest::StatusCode::TOO_MANY_REQUESTS || s.as_u16() >= 500 => {
                StorageError::ServiceUnavailable(format!("GCS returned {status}: {body}"))
            }
            _ => StorageError::Internal(format!("GCS error {status}: {body}")),
        }
    }

    /// Download a GCS object and return it as a streaming reader.
    async fn download_to_file(
        &self,
        object_name: &str,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let token = self.auth.get_token().await?;

        let resp = self
            .gcs_request(
                reqwest::Method::GET,
                &format!("/b/{}/o/{object_name}?alt=media", self.bucket),
            )
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| StorageError::Internal(format!("GCS request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.map_err(|e| {
                StorageError::Internal(format!("Failed to read error response: {e}"))
            })?;
            return Err(Self::map_http_error(status, object_name, &body));
        }

        let stream = resp
            .bytes_stream()
            .map(|result| result.map_err(std::io::Error::other));

        let reader = StreamReader::new(stream);

        Ok(Box::new(reader))
    }

    /// Download a GCS object starting from the given byte offset.
    async fn download_to_file_with_range(
        &self,
        object_name: &str,
        offset: u64,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let token = self.auth.get_token().await?;

        let resp = self
            .gcs_request(
                reqwest::Method::GET,
                &format!("/b/{}/o/{object_name}?alt=media", self.bucket),
            )
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header("Range", format!("bytes={offset}-"))
            .send()
            .await
            .map_err(|e| StorageError::Internal(format!("GCS request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.map_err(|e| {
                StorageError::Internal(format!("Failed to read error response: {e}"))
            })?;
            return Err(Self::map_http_error(status, object_name, &body));
        }

        let stream = resp
            .bytes_stream()
            .map(|result| result.map_err(std::io::Error::other));

        let reader = StreamReader::new(stream);

        Ok(Box::new(reader))
    }
}

#[async_trait::async_trait]
impl Storage for GcsStorage {
    /// Check if a file or directory exists at the given path.
    async fn exists(&self, path: &Path) -> bool {
        self.metadata(path).await.is_ok()
    }

    /// Check if the path points to a file.
    async fn is_file(&self, path: &Path) -> bool {
        match self.metadata(path).await {
            Ok(meta) => meta.is_file,
            Err(_) => false,
        }
    }

    /// Check if the path points to a directory (virtual directory marker).
    async fn is_dir(&self, path: &Path) -> bool {
        let object_name = Self::path_to_object_name(path);

        // Build the directory marker object name.
        let marker = if object_name.ends_with('/') {
            object_name.clone()
        } else {
            format!("{object_name}/")
        };

        // Check if the marker object exists.
        let Ok(token) = self.auth.get_token().await else {
            return false;
        };

        if self
            .gcs_request(
                reqwest::Method::GET,
                &format!("/b/{}/o/{marker}", self.bucket),
            )
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .send()
            .await
            .is_ok()
        {
            return true;
        }

        // Fall back to listing - if there are sub-entries, it's a directory.
        self.list(path)
            .await
            .is_ok_and(|entries| !entries.is_empty())
    }

    /// Read the entire contents of a file into bytes.
    async fn read(&self, path: &Path) -> Result<Vec<u8>> {
        let object_name = Self::path_to_object_name(path);

        let token = self.auth.get_token().await?;

        let resp = self
            .gcs_request(
                reqwest::Method::GET,
                &format!("/b/{}/o/{object_name}?alt=media", self.bucket),
            )
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| StorageError::Internal(format!("GCS request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.map_err(|e| {
                StorageError::Internal(format!("Failed to read error response: {e}"))
            })?;
            return Err(Self::map_http_error(status, &object_name, &body));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| StorageError::Internal(format!("Failed to read response body: {e}")))?;

        Ok(bytes.to_vec())
    }

    /// Open a file and return a streaming reader.
    async fn open(&self, path: &Path) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let object_name = Self::path_to_object_name(path);
        self.download_to_file(&object_name).await
    }

    async fn seek_read(
        &self,
        path: &Path,
        offset: u64,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let object_name = Self::path_to_object_name(path);
        self.download_to_file_with_range(&object_name, offset).await
    }

    /// Write bytes to a file, creating it if it doesn't exist.
    async fn write(&self, path: &Path, contents: &[u8]) -> Result<()> {
        let object_name = Self::path_to_object_name(path);

        let token = self.auth.get_token().await?;

        let encoded_name =
            percent_encoding::utf8_percent_encode(&object_name, percent_encoding::NON_ALPHANUMERIC)
                .to_string();

        let resp = self
            .gcs_request(
                reqwest::Method::POST,
                &format!("/b/{}/o?uploadType=media&name={encoded_name}", self.bucket),
            )
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(contents.to_vec())
            .send()
            .await
            .map_err(|e| StorageError::Internal(format!("GCS request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.map_err(|e| {
                StorageError::Internal(format!("Failed to read error response: {e}"))
            })?;
            return Err(Self::map_http_error(status, &object_name, &body));
        }

        Ok(())
    }

    /// Append bytes to a file, creating it if it doesn't exist.
    ///
    /// GCS does not support true append semantics. This reads the existing
    /// content, appends the new bytes, and writes back the combined result.
    /// This is not atomic and should be used with caution in concurrent
    /// environments.
    async fn append(&self, path: &Path, contents: &[u8]) -> Result<()> {
        let existing = self.read(path).await?;
        let mut combined = existing;
        combined.extend_from_slice(contents);
        self.write(path, &combined).await
    }

    /// Delete a file.
    async fn delete(&self, path: &Path) -> Result<()> {
        let object_name = Self::path_to_object_name(path);

        let token = self.auth.get_token().await?;

        let encoded_name =
            percent_encoding::utf8_percent_encode(&object_name, percent_encoding::NON_ALPHANUMERIC)
                .to_string();

        let resp = self
            .gcs_request(
                reqwest::Method::DELETE,
                &format!("/b/{}/o/{encoded_name}", self.bucket),
            )
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| StorageError::Internal(format!("GCS request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.map_err(|e| {
                StorageError::Internal(format!("Failed to read error response: {e}"))
            })?;
            return Err(Self::map_http_error(status, &object_name, &body));
        }

        Ok(())
    }

    /// Get metadata for a file or directory.
    async fn metadata(&self, path: &Path) -> Result<FileMetadata> {
        let object_name = Self::path_to_object_name(path);

        let token = self.auth.get_token().await?;

        let encoded_name =
            percent_encoding::utf8_percent_encode(&object_name, percent_encoding::NON_ALPHANUMERIC)
                .to_string();

        let resp = self
            .gcs_request(
                reqwest::Method::GET,
                &format!("/b/{}/o/{encoded_name}", self.bucket),
            )
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| StorageError::Internal(format!("GCS request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.map_err(|e| {
                StorageError::Internal(format!("Failed to read error response: {e}"))
            })?;
            return Err(Self::map_http_error(status, &object_name, &body));
        }

        let text = resp
            .text()
            .await
            .map_err(|e| StorageError::Internal(format!("Failed to read GCS response: {e}")))?;

        let metadata: GcsObjectMetadata = serde_json::from_str(&text)
            .map_err(|e| StorageError::Internal(format!("Failed to parse GCS metadata: {e}")))?;

        let is_file = !metadata.name.ends_with('/');
        let size = metadata.parse_size();
        let created = GcsObjectMetadata::parse_timestamp(metadata.time_created.as_ref());
        let modified = GcsObjectMetadata::parse_timestamp(metadata.updated.as_ref());

        Ok(FileMetadata {
            name: metadata.name,
            is_file,
            size,
            created,
            modified,
        })
    }

    /// List entries in a directory.
    async fn list(&self, dir: &Path) -> Result<Vec<DirEntry>> {
        let dir_str = dir.to_string_lossy().to_string();
        let gcs_prefix = dir_str.strip_prefix('/').unwrap_or(&dir_str).to_string();

        // Strip trailing slash for the prefix but ensure we stay within the directory.
        let prefix = gcs_prefix
            .strip_suffix('/')
            .unwrap_or(&gcs_prefix)
            .to_string();

        let token = self.auth.get_token().await?;

        let resp = self
            .gcs_request(
                reqwest::Method::GET,
                &format!(
                    "/b/{}/o?prefix={}&delimiter=/&maxResults=1000",
                    self.bucket,
                    percent_encoding::utf8_percent_encode(
                        &prefix,
                        percent_encoding::NON_ALPHANUMERIC,
                    )
                ),
            )
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| StorageError::Internal(format!("GCS request failed: {e}")))?;

        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if !status.is_success() {
            let body = resp.text().await.map_err(|e| {
                StorageError::Internal(format!("Failed to read list response: {e}"))
            })?;
            return Err(Self::map_http_error(status, &dir_str, &body));
        }

        let text = resp
            .text()
            .await
            .map_err(|e| StorageError::Internal(format!("Failed to read GCS response: {e}")))?;

        let list_resp: GcsListResponse = serde_json::from_str(&text).map_err(|e| {
            StorageError::Internal(format!("Failed to parse GCS list response: {e}"))
        })?;

        let mut entries = Vec::new();

        // Add directory entries from common prefixes.
        if let Some(prefixes) = list_resp.prefixes {
            for prefix in prefixes {
                // Remove trailing slash from prefix name.
                let dir_name = prefix.trim_end_matches('/');

                // Split the prefix to get the immediate child directory name.
                let name = dir_name.rsplit('/').next().unwrap_or(dir_name).to_string();

                // Skip directory markers (our .dir/ convention).
                if name.ends_with("/.dir") || name == ".dir" {
                    continue;
                }

                let full_path = PathBuf::from("/").join(&dir_str).join(&name);
                entries.push(DirEntry::new(name, true, 0).with_path(full_path));
            }
        }

        // Add file entries from items.
        if let Some(items) = list_resp.items {
            for item in items {
                // Skip directory markers (our .dir/ convention).
                if item.name.ends_with("/.dir/") || item.name.ends_with("/.dir") {
                    continue;
                }

                // Strip the directory prefix from the item name to get the file name.
                let item_name = if let Some(stripped) = item.name.strip_prefix(&prefix) {
                    stripped.trim_start_matches('/').to_string()
                } else {
                    item.name.clone()
                };

                let full_path = PathBuf::from("/").join(&dir_str).join(&item_name);
                entries.push(
                    DirEntry::new(item_name, false, item.parse_size())
                        .with_path(full_path)
                        .with_modified(GcsObjectMetadata::parse_timestamp(item.updated.as_ref())),
                );
            }
        }

        Ok(entries)
    }

    /// Create a directory (virtual - creates a directory marker object).
    ///
    /// Returns an error if the parent directory doesn't exist (in GCS terms,
    /// if no objects exist under the parent prefix).
    async fn create_dir(&self, path: &Path) -> Result<()> {
        let object_name = Self::path_to_object_name(path);

        let dir_name = if object_name.ends_with('/') {
            format!("{object_name}.dir/")
        } else {
            format!("{object_name}/.dir/")
        };

        self.write(Path::new(&dir_name), &[]).await
    }

    /// Create a directory and all its parents (virtual directories).
    ///
    /// Creates directory marker objects for each directory level.
    async fn create_dir_all(&self, path: &Path) -> Result<()> {
        let object_name = Self::path_to_object_name(path);

        // Strip trailing slash for processing.
        let clean = object_name.strip_suffix('/').unwrap_or(&object_name);

        // Create a directory marker for each path component.
        let mut current = String::new();
        for component in clean.split('/') {
            if component.is_empty() {
                continue;
            }
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(component);

            // Create marker for this directory level.
            let marker_path = format!("{current}/.dir/");
            self.write(Path::new(&marker_path), &[]).await?;
        }

        Ok(())
    }

    /// Remove an empty directory.
    ///
    /// Returns `StorageError::DirectoryNotEmpty` if the directory contains objects.
    async fn remove_dir(&self, path: &Path) -> Result<()> {
        // List all entries under this directory.
        let entries = self.list(path).await?;

        // Check for real content (files and subdirectories).
        let has_content = entries
            .iter()
            .any(|e| !e.is_dir || !e.name.ends_with("/.dir"));

        if has_content {
            return Err(StorageError::DirectoryNotEmpty(path.to_path_buf()));
        }

        // Delete all entries including directory markers.
        for entry in &entries {
            self.delete(&entry.path).await?;
        }

        Ok(())
    }

    /// Remove a directory and all its contents recursively.
    async fn remove_dir_all(&self, path: &Path) -> Result<()> {
        let entries = self.list(path).await?;

        for entry in &entries {
            self.delete(&entry.path).await?;
        }

        Ok(())
    }

    /// Rename or move a file or directory within the same bucket.
    ///
    /// Implements rename as copy-then-delete since GCS has no native rename.
    async fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        self.copy(from, to).await?;
        self.delete(from).await
    }

    /// Copy a file to a new location within the same bucket.
    async fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        let from_name = Self::path_to_object_name(from);
        let to_name = Self::path_to_object_name(to);

        let token = self.auth.get_token().await?;

        let from_encoded =
            percent_encoding::utf8_percent_encode(&from_name, percent_encoding::NON_ALPHANUMERIC)
                .to_string();

        let to_encoded =
            percent_encoding::utf8_percent_encode(&to_name, percent_encoding::NON_ALPHANUMERIC)
                .to_string();

        let resp = self
            .gcs_request(
                reqwest::Method::POST,
                &format!(
                    "/b/{}/o/{from_encoded}?destination={to_encoded}",
                    self.bucket
                ),
            )
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(CONTENT_TYPE, "application/json")
            .send()
            .await
            .map_err(|e| StorageError::Internal(format!("GCS request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.map_err(|e| {
                StorageError::Internal(format!("Failed to read error response: {e}"))
            })?;
            return Err(Self::map_http_error(status, &to_name, &body));
        }

        Ok(())
    }

    /// Canonicalize a path to its GCS URI form.
    ///
    /// Returns `gs://{bucket}/{path}`.
    async fn canonicalize(&self, path: &Path) -> Result<PathBuf> {
        let object_name = Self::path_to_object_name(path);
        let uri = format!("gs://{}/{}", self.bucket, object_name);
        Ok(PathBuf::from(uri))
    }

    /// Get the size of a file in bytes.
    async fn size(&self, path: &Path) -> Result<u64> {
        let meta = self.metadata(path).await?;
        Ok(meta.size)
    }
}

#[cfg(all(test, feature = "gcs"))]
/// Integration tests for the GCS storage backend.
mod tests {
    use super::*;

    #[test]
    /// Tests path-to-object-name conversion for various input paths.
    fn test_path_to_object_name() {
        assert_eq!(
            GcsStorage::path_to_object_name(Path::new("/foo/bar.txt")),
            "foo/bar.txt"
        );
        assert_eq!(
            GcsStorage::path_to_object_name(Path::new("relative/path.txt")),
            "relative/path.txt"
        );
        assert_eq!(GcsStorage::path_to_object_name(Path::new("/")), "");
        assert_eq!(
            GcsStorage::path_to_object_name(Path::new("/a/b/c")),
            "a/b/c"
        );
    }

    #[test]
    /// Tests that 404 status maps to `StorageError::NotFound`.
    fn test_map_http_error_not_found() {
        let err =
            GcsStorage::map_http_error(reqwest::StatusCode::NOT_FOUND, "test.txt", "Not Found");
        assert!(matches!(err, StorageError::NotFound(p) if p.to_string_lossy() == "test.txt"));
    }

    #[test]
    /// Tests that 403 status maps to `StorageError::Authentication`.
    fn test_map_http_error_forbidden() {
        let err =
            GcsStorage::map_http_error(reqwest::StatusCode::FORBIDDEN, "test.txt", "Access Denied");
        assert!(matches!(err, StorageError::Authentication(_)));
    }

    #[test]
    /// Tests loading credentials from a raw JSON service account key.
    fn test_load_credentials_from_json() {
        let json = r#"{"client_email": "test@example.com", "private_key": "-----BEGIN RSA PRIVATE KEY-----\ntest\n-----END RSA PRIVATE KEY-----"}"#;
        let result = GcsStorage::load_credentials(json);
        assert!(result.is_ok());
        let creds = result.unwrap();
        assert_eq!(creds.client_email, "test@example.com");
    }
}

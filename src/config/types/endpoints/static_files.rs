//! Static files action configuration types.
use serde::Deserialize;

use crate::config::types::DEFAULT_CACHE_MAX_AGE;

use super::common::{
    default_cache_max_age, default_index, default_true, deserialize_allowed_extensions,
};

/// Per-extension Cache-Control rule.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
/// `CacheRuleConfig`
pub struct CacheRuleConfig {
    /// File extensions to match (e.g. `` `[".html", ".js"]` ``).
    pub extensions: Vec<String>,
    /// Cache-Control header value (e.g. "public, max-age=31536000, immutable").
    pub cache_control: String,
}

/// Static file serving configuration for an endpoint.
#[allow(clippy::struct_excessive_bools)] // each bool field is an independent YAML config option
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `StaticFilesConfig`
pub struct StaticFilesConfig {
    /// Named store to use (must match a key in `stores`).
    pub storage: String,
    /// Index file name (e.g. "index.html").
    #[serde(default = "default_index")]
    pub index: String,
    /// Whether to generate directory listings.
    #[serde(default)]
    pub directory_listing: bool,
    /// Cache-Control max-age in seconds.
    #[serde(default = "default_cache_max_age")]
    pub cache_max_age: u64,
    /// `ETag` generation for cache validation.
    #[serde(default = "default_true")]
    pub etag: bool,
    /// Support for HTTP range requests (partial content / 206).
    #[serde(default = "default_true")]
    pub range_requests: bool,
    /// Support for HTTP HEAD method.
    #[serde(default = "default_true")]
    pub head_support: bool,
    /// Per-extension Cache-Control rules. Overrides `cache_max_age` for
    /// matching file extensions.
    #[serde(default)]
    pub cache_rules: Vec<CacheRuleConfig>,
    /// File upload configuration (optional).
    #[serde(default)]
    pub upload: Option<UploadConfig>,
    /// Image resize configuration (optional).
    #[serde(default)]
    pub image_resize: Option<ImageResizeConfig>,
    /// Streaming configuration (optional).
    #[serde(default)]
    pub streaming: Option<StreamingConfig>,
}

impl Default for StaticFilesConfig {
    /// Returns a static files configuration with default index page, cache, and range request settings.
    fn default() -> Self {
        Self {
            storage: String::new(),
            index: "index.html".to_string(),
            directory_listing: false,
            cache_max_age: DEFAULT_CACHE_MAX_AGE,
            etag: true,
            range_requests: true,
            head_support: true,
            cache_rules: Vec::new(),
            upload: None,
            image_resize: None,
            streaming: None,
        }
    }
}

/// MIME type detection method for uploads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
/// `UploadMimeDetection`
pub enum UploadMimeDetection {
    /// Detect MIME type from file extension.
    Extension,
    /// Detect MIME type by inspecting magic bytes.
    #[default]
    Magic,
}

/// File upload configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `UploadConfig`
pub struct UploadConfig {
    /// Whether uploads are enabled.
    pub enabled: bool,
    /// Maximum upload size in bytes.
    pub max_size: u64,
    /// Allowed file extensions (empty = all).
    /// Leading dots are stripped during parsing, so both `.jpg` and `jpg` work.
    #[serde(deserialize_with = "deserialize_allowed_extensions")]
    pub allowed_extensions: Vec<String>,
    /// Subdirectory pattern for organizing uploads.
    /// Available placeholders: {`user_id`}, {year}, {month}, {day}, {uuid}
    pub create_subdirectory: Option<String>,
    /// MIME type detection method.
    #[serde(default)]
    pub mime_detection: UploadMimeDetection,
}

impl Default for UploadConfig {
    /// Returns an upload configuration with uploads disabled and magic MIME detection.
    fn default() -> Self {
        Self {
            enabled: false,
            max_size: 10 * 1024 * 1024, // 10 MiB
            allowed_extensions: Vec::new(),
            create_subdirectory: None,
            mime_detection: UploadMimeDetection::Magic,
        }
    }
}

/// Image resize fit mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
/// `ImageResizeFit`
pub enum ImageResizeFit {
    /// Scale down to fit within bounds, preserving aspect ratio.
    #[default]
    ScaleDown,
    /// Fill the bounds, cropping if necessary.
    Cover,
    /// Fit entirely within the bounds, preserving aspect ratio.
    Contain,
}

/// Image resize configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `ImageResizeConfig`
pub struct ImageResizeConfig {
    /// Whether on-demand resizing is enabled.
    pub enabled: bool,
    /// Maximum dimension for auto-resize.
    pub max_dimension: usize,
    /// Supported formats for conversion.
    pub supported_formats: Vec<String>,
    /// Default resize fit mode.
    #[serde(default)]
    pub default_fit: ImageResizeFit,
    /// Cache directory for resized images (optional).
    /// Defaults to "cache/resized" under the root directory.
    #[serde(default)]
    pub cache_dir: Option<String>,
}

impl Default for ImageResizeConfig {
    /// Returns an image resize configuration with resizing disabled and default format support.
    fn default() -> Self {
        Self {
            enabled: false,
            max_dimension: super::common::IMAGE_MAX_DIMENSION,
            supported_formats: vec![
                "jpg".to_string(),
                "jpeg".to_string(),
                "png".to_string(),
                "webp".to_string(),
            ],
            default_fit: ImageResizeFit::ScaleDown,
            cache_dir: None,
        }
    }
}

/// Streaming configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// `StreamingConfig`
pub struct StreamingConfig {
    /// Enable chunked streaming for large files.
    pub enabled: bool,
    /// Chunk size in bytes (default: 64 KiB).
    pub buffer_size: usize,
    /// Threshold for enabling streaming (files > this size use streaming).
    pub threshold: u64,
    /// Whether to include Content-Length header in streaming responses.
    #[serde(default = "default_true")]
    pub include_content_length: bool,
}

impl Default for StreamingConfig {
    /// Returns a streaming configuration with streaming disabled and 64 KiB buffer size.
    fn default() -> Self {
        Self {
            enabled: false,
            buffer_size: 65536,     // 64 KiB
            threshold: 1024 * 1024, // 1 MiB
            include_content_length: true,
        }
    }
}

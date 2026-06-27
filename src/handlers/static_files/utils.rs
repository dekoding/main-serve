use std::path::Path;

use axum::http::HeaderValue;
use axum::response::Response;
use http::header;

use crate::config::types::CacheRuleConfig;

/// Apply Content-Type and Cache-Control headers to a response.
pub fn apply_static_headers(
    response: &mut Response,
    content_type: &str,
    cache_max_age: u64,
    path: Option<&Path>,
    cache_rules: &[CacheRuleConfig],
) {
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    let cache_control = match (path, cache_rules.is_empty()) {
        (Some(p), false) => get_cache_control(p, cache_max_age, cache_rules),
        _ => format!("public, max-age={cache_max_age}"),
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str(&cache_control)
            .unwrap_or(HeaderValue::from_static("public, max-age=3600")),
    );
}

/// Get the Cache-Control header value for a path.
///
/// Checks extension-specific cache rules first, then falls back to the
/// default max-age.
///
/// Cache rule extensions may include or omit the leading dot (e.g. ".js" or "js").
/// File path extensions from `Path::extension()` do not include the dot.
pub fn get_cache_control(
    path: &Path,
    default_max_age: u64,
    cache_rules: &[CacheRuleConfig],
) -> String {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    for rule in cache_rules {
        if rule.extensions.iter().any(|e| {
            let rule_ext = e.trim_start_matches('.');
            rule_ext == ext
        }) {
            return rule.cache_control.clone();
        }
    }

    format!("public, max-age={default_max_age}")
}

/// Determine MIME type from file extension.
#[must_use]
pub fn mime_from_path(path: &Path) -> String {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "pdf" => "application/pdf",
        "xml" => "application/xml; charset=utf-8",
        "txt" | "text" | "md" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        "webp" => "image/webp",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Minimal HTML escaping for safe inclusion in generated HTML.
#[must_use]
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Format a byte count as a human-readable size string.
#[must_use]
pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    for unit in UNITS {
        if size < 1024.0 {
            return if *unit == "B" {
                format!("{size:.0} {unit}")
            } else {
                format!("{size:.1} {unit}")
            };
        }
        size /= 1024.0;
    }
    format!("{size:.1} PiB")
}

/// Format a `SystemTime` as YYYY-MM-DD HH:MM.
#[must_use]
pub fn format_modified(time: std::time::SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Local> = time.into();
    dt.format("%Y-%m-%d %H:%M").to_string()
}

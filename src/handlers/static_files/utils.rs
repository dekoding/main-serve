use std::path::Path;

use axum::http::HeaderValue;
use axum::response::Response;
use http::header;

use crate::error::AppError;

/// Apply Content-Type and Cache-Control headers to a response.
pub fn apply_static_headers(response: &mut Response, content_type: &str, cache_max_age: u64) {
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str(&format!("public, max-age={cache_max_age}"))
            .unwrap_or(HeaderValue::from_static("public, max-age=3600")),
    );
}

/// Determine MIME type from file extension.
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
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Format a byte count as a human-readable size string.
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

/// Format a SystemTime as YYYY-MM-DD HH:MM.
pub fn format_modified(time: std::time::SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Local> = time.into();
    dt.format("%Y-%m-%d %H:%M").to_string()
}

/// Extract authentication info from request for file operations.
/// This is a helper to be used by the router when calling file handlers.
pub async fn extract_auth_for_files(
    state: &crate::server::state::AppState,
    endpoint: &crate::config::types::EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &std::collections::HashMap<String, String>,
) -> Result<crate::auth::middleware::AuthInfo, AppError> {
    if endpoint.auth == "none" {
        return Ok(crate::auth::middleware::AuthInfo::default());
    }
    crate::auth::middleware::authenticate(
        &endpoint.auth,
        &state.config.read().await.auth,
        headers,
        query_params,
    )
    .await
}

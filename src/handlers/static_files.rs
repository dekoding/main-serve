/// Static file serving handler with SPA fallback and directory listing support.
///
/// Serves files from a configured root directory. Supports:
/// - Index file serving (e.g. `index.html`)
/// - Cache-Control headers
/// - SPA fallback (serve index.html for non-file routes)
/// - Directory listing (optional)
use std::path::Path;

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

use crate::config::types::EndpointConfig;
use crate::error::AppError;
use crate::server::state::AppState;

/// Apply Content-Type and Cache-Control headers to a response.
fn apply_static_headers(response: &mut Response, content_type: &str, cache_max_age: u64) {
    response.headers_mut().insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        http::header::CACHE_CONTROL,
        HeaderValue::from_str(&format!("public, max-age={cache_max_age}"))
            .unwrap_or(HeaderValue::from_static("public, max-age=3600")),
    );
}

/// Handle a static file endpoint - serves files from the configured root directory.
///
/// # Errors
///
/// Returns `AppError::Internal` if the static config is missing or the root
/// directory does not exist. Returns `AppError::NotFound` if the requested
/// file cannot be found. Returns `AppError::Forbidden` on path traversal attempts.
pub async fn handle_static_files(
    State(_state): State<AppState>,
    uri: Uri,
    endpoint: EndpointConfig,
) -> Result<Response, AppError> {
    let static_config = endpoint.static_files.as_ref().ok_or_else(|| {
        AppError::Internal("static_files config missing on static endpoint".to_string())
    })?;

    let root = std::path::Path::new(&static_config.root);
    if !root.exists() {
        return Err(AppError::Internal(format!(
            "Static file root '{}' does not exist",
            static_config.root
        )));
    }

    // Determine the file path relative to the endpoint path.
    let request_path = uri.path();
    let ep_path = endpoint
        .path
        .trim_end_matches("/*")
        .trim_end_matches("{*rest}");
    let relative = request_path
        .strip_prefix(ep_path)
        .unwrap_or(request_path)
        .trim_start_matches('/');

    let relative = percent_encoding::percent_decode_str(relative)
        .decode_utf8()
        .map_err(|_| AppError::BadRequest("Invalid UTF-8 in path".to_string()))?
        .into_owned();

    // Security: reject any path components that attempt traversal before
    // touching the filesystem. This catches both existing and non-existing paths.
    if relative.split('/').any(|seg| seg == ".." || seg == ".") {
        return Err(AppError::Forbidden("Path traversal denied".to_string()));
    }

    let resolved = if relative.is_empty() {
        root.to_path_buf()
    } else {
        let r = root.join(relative);
        // Double-check with canonicalize for symlink-based escapes.
        if let (Ok(canonical), Ok(root_canonical)) = (
            tokio::fs::canonicalize(&r).await,
            tokio::fs::canonicalize(root).await,
        ) && !canonical.starts_with(&root_canonical)
        {
            return Err(AppError::Forbidden("Path traversal denied".to_string()));
        }
        r
    };

    // If the resolved path is a directory, try the index file first.
    // If the index file doesn't exist and directory_listing is enabled,
    // generate a listing. Otherwise fall through to SPA fallback or 404.
    let resolved_meta = tokio::fs::metadata(&resolved).await.ok();
    if resolved_meta
        .as_ref()
        .is_some_and(std::fs::Metadata::is_dir)
    {
        let index_path = resolved.join(&static_config.index);
        let index_exists = tokio::fs::metadata(&index_path)
            .await
            .is_ok_and(|m| m.is_file());
        if index_exists {
            return serve_file(&index_path, static_config.cache_max_age).await;
        }
        if static_config.directory_listing {
            return generate_directory_listing(&resolved, request_path).await;
        }
        if static_config.spa_fallback {
            return serve_spa_fallback(root, &static_config.index, static_config.cache_max_age)
                .await;
        }
        return Err(AppError::NotFound(format!(
            "File not found: {request_path}"
        )));
    }

    // Try to serve the file.
    match tokio::fs::read(&resolved).await {
        Ok(contents) => {
            let mut response = (StatusCode::OK, contents).into_response();
            apply_static_headers(
                &mut response,
                &mime_from_path(&resolved),
                static_config.cache_max_age,
            );
            Ok(response)
        }
        Err(_) if static_config.spa_fallback => {
            serve_spa_fallback(root, &static_config.index, static_config.cache_max_age).await
        }
        Err(_) => Err(AppError::NotFound(format!(
            "File not found: {request_path}"
        ))),
    }
}

/// Determine MIME type from file extension.
fn mime_from_path(path: &Path) -> String {
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

/// Serve a single file with appropriate Content-Type and Cache-Control headers.
async fn serve_file(path: &Path, cache_max_age: u64) -> Result<Response, AppError> {
    let contents = tokio::fs::read(path)
        .await
        .map_err(|e| AppError::NotFound(format!("File not found: {e}")))?;
    let mut response = (StatusCode::OK, contents).into_response();
    apply_static_headers(&mut response, &mime_from_path(path), cache_max_age);
    Ok(response)
}

/// Serve the index file as a SPA fallback.
async fn serve_spa_fallback(
    root: &Path,
    index: &str,
    cache_max_age: u64,
) -> Result<Response, AppError> {
    let index_path = root.join(index);
    let contents = tokio::fs::read(&index_path)
        .await
        .map_err(|e| AppError::NotFound(format!("Index file not found: {e}")))?;
    let mut response = (StatusCode::OK, contents).into_response();
    apply_static_headers(&mut response, &mime_from_path(&index_path), cache_max_age);
    Ok(response)
}

/// Metadata collected for a single directory entry.
struct DirEntryInfo {
    name: String,
    is_dir: bool,
    size: u64,
    modified: Option<std::time::SystemTime>,
    mode: u32,
    uid: u32,
    gid: u32,
}

/// Generate an HTML directory listing for the given directory.
async fn generate_directory_listing(dir: &Path, request_path: &str) -> Result<Response, AppError> {
    let mut entries = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read directory: {e}")))?;

    let mut items: Vec<DirEntryInfo> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry.metadata().await.ok();
        let is_dir = metadata
            .as_ref()
            .map(std::fs::Metadata::is_dir)
            .unwrap_or(false);
        let size = metadata.as_ref().map(std::fs::Metadata::len).unwrap_or(0);
        let modified = metadata.as_ref().and_then(|m| m.modified().ok());
        items.push(DirEntryInfo {
            name,
            is_dir,
            size,
            modified,
            mode: metadata
                .as_ref()
                .map(std::os::unix::fs::MetadataExt::mode)
                .unwrap_or(0),
            uid: metadata
                .as_ref()
                .map(std::os::unix::fs::MetadataExt::uid)
                .unwrap_or(0),
            gid: metadata
                .as_ref()
                .map(std::os::unix::fs::MetadataExt::gid)
                .unwrap_or(0),
        });
    }
    items.sort_by(|a, b| {
        // Directories first, then alphabetical.
        b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name))
    });

    let decoded_path = percent_encoding::percent_decode_str(request_path)
        .decode_utf8()
        .map_err(|_| AppError::BadRequest("Invalid UTF-8 in path".to_string()))?
        .into_owned();
    let path_display = html_escape(&decoded_path);

    let mut html = format!(
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>Index of {path_display}</title>\
         <style>\
         body{{font-family:sans-serif;padding:1em}}\
         a{{text-decoration:none}}a:hover{{text-decoration:underline}}\
         .dir{{font-weight:bold}}\
         table{{border-collapse:collapse;width:100%}}\
         th,td{{text-align:left;padding:0.3em 1em;border-bottom:1px solid #ddd}}\
         th{{border-bottom:2px solid #999}}\
         td.size,th.size{{text-align:right}}\
         tr:hover{{background:#f5f5f5}}\
         .perms{{font-family:monospace}}\
         </style></head>\n<body>\n<h1>Index of {path_display}</h1>\n"
    );

    html.push_str("<table>\n<tr>");
    html.push_str("<th>Permissions</th><th>Owner</th><th>Group</th>");
    html.push_str("<th class=\"size\">Size</th><th>Modified</th><th>Name</th></tr>\n");

    // Parent directory link (unless at root).
    if request_path != "/" {
        html.push_str("<tr>");
        html.push_str("<td></td><td></td><td></td>");
        html.push_str("<td></td><td></td><td><a href=\"../\">..</a></td></tr>\n");
    }

    // Ensure the base path ends with exactly one slash for link construction.
    let base = if request_path.ends_with('/') {
        request_path.to_string()
    } else {
        format!("{request_path}/")
    };

    for item in &items {
        let escaped_name = html_escape(&item.name);
        let link = if item.is_dir {
            format!("<a class=\"dir\" href=\"{base}{escaped_name}/\">{escaped_name}/</a>")
        } else {
            format!("<a href=\"{base}{escaped_name}\">{escaped_name}</a>")
        };

        let size_str = if item.is_dir {
            "-".to_string()
        } else {
            format_size(item.size)
        };
        let modified_str = item
            .modified
            .map(format_modified)
            .unwrap_or_else(|| "-".to_string());

        html.push_str("<tr>");
        {
            let perms = format_permissions(item.mode, item.is_dir);
            let owner = resolve_username(item.uid);
            let group = resolve_group(item.gid);
            html.push_str(&format!(
                "<td class=\"perms\">{perms}</td><td>{}</td><td>{}</td>",
                html_escape(&owner),
                html_escape(&group)
            ));
        }
        html.push_str(&format!(
            "<td class=\"size\">{size_str}</td><td>{modified_str}</td><td>{link}</td></tr>\n"
        ));
    }

    html.push_str("</table>\n</body></html>\n");

    let mut response = (StatusCode::OK, html).into_response();
    apply_static_headers(&mut response, "text/html; charset=utf-8", 0);
    Ok(response)
}

/// Format a Unix mode into a `drwxrwxrwx`-style permission string.
fn format_permissions(mode: u32, is_dir: bool) -> String {
    let mut s = String::with_capacity(10);
    s.push(if is_dir { 'd' } else { '-' });
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    s
}

/// Resolve a numeric uid to a username, falling back to the numeric string.
fn resolve_username(uid: u32) -> String {
    nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid))
        .ok()
        .flatten()
        .map(|u| u.name)
        .unwrap_or_else(|| uid.to_string())
}

/// Resolve a numeric gid to a group name, falling back to the numeric string.
fn resolve_group(gid: u32) -> String {
    nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(gid))
        .ok()
        .flatten()
        .map(|u| u.name)
        .unwrap_or_else(|| gid.to_string())
}

/// Format a byte count as a human-readable size string.
fn format_size(bytes: u64) -> String {
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

/// Format a `SystemTime` as `YYYY-MM-DD HH:MM`.
fn format_modified(time: std::time::SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Local> = time.into();
    dt.format("%Y-%m-%d %H:%M").to_string()
}

/// Minimal HTML escaping for safe inclusion in generated HTML.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

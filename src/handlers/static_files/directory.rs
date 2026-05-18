use std::path::Path;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;

use crate::error::AppError;
use crate::handlers::static_files::utils::{apply_static_headers, format_size, html_escape};
use crate::storage::Storage;
use percent_encoding::percent_decode_str;

/// Metadata collected for a single directory entry.
#[derive(Debug)]
pub struct DirEntryInfo {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}

/// Generate an HTML directory listing for the given directory.
pub async fn generate_directory_listing(
    storage: &dyn Storage,
    dir: &Path,
    request_path: &str,
) -> Result<Response, AppError> {
    let entries = storage
        .list(dir)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read directory: {e}")))?;

    let mut items: Vec<DirEntryInfo> = Vec::new();
    for entry in entries {
        let is_dir = !entry.is_file();
        let size = entry.size;
        items.push(DirEntryInfo {
            name: entry.name,
            is_dir,
            size,
            modified: None,
            mode: 0,
            uid: 0,
            gid: 0,
        });
    }
    items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));

    let decoded_path = percent_decode_str(request_path)
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

    if request_path != "/" {
        html.push_str("<tr>");
        html.push_str("<td></td><td></td><td></td>");
        html.push_str("<td></td><td></td><td><a href=\"../\">..</a></td></tr>\n");
    }

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

        html.push_str("<tr>");
        html.push_str(&format!(
            "<td class=\"size\">{size_str}</td><td>{link}</td></tr>\n"
        ));
    }

    html.push_str("</table>\n</body></html>\n");

    let mut response = (StatusCode::OK, html).into_response();
    apply_static_headers(&mut response, "text/html; charset=utf-8", 0);
    Ok(response)
}

/// Format a Unix mode into a `drwxrwxrwx`-style permission string.
pub fn format_permissions(mode: u32, is_dir: bool) -> String {
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
pub fn resolve_username(uid: u32) -> String {
    nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid))
        .ok()
        .flatten()
        .map_or_else(|| uid.to_string(), |u| u.name)
}

/// Resolve a numeric gid to a group name, falling back to the numeric string.
pub fn resolve_group(gid: u32) -> String {
    nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(gid))
        .ok()
        .flatten()
        .map_or_else(|| gid.to_string(), |u| u.name)
}

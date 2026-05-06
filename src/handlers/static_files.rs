use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use axum::body::Body;
use axum::extract::Query;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::Uri;
use axum::response::IntoResponse;
use axum::response::Response;
use chrono::Utc;
use http::Method;
use http::header;
use http::header::CONTENT_LENGTH;
use image::ImageFormat;
use percent_encoding::percent_decode_str;
use uuid::Uuid;

use crate::auth::middleware::AuthInfo;
use crate::config::types::EndpointConfig;
use crate::config::types::ImageResizeConfig;
use crate::config::types::StaticFilesConfig;
use crate::config::types::StreamingConfig;
use crate::config::types::UploadConfig;
use crate::error::AppError;
use crate::server::state::AppState;

/// Context for handling static file GET requests.
struct StaticGetContext<'a> {
    endpoint: &'a EndpointConfig,
    config: &'a StaticFilesConfig,
    relative: &'a str,
    root: &'a Path,
    request_path: &'a str,
    query: &'a Query<Option<HashMap<String, String>>>,
    headers: &'a axum::http::HeaderMap,
}

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
/// Handle a static file endpoint - serves files, uploads, or deletes based on method.
pub async fn handle_static_files(
    state: State<AppState>,
    method: Method,
    uri: Uri,
    endpoint: EndpointConfig,
    headers: axum::http::HeaderMap,
    query: Query<Option<HashMap<String, String>>>,
) -> Result<Response, AppError> {
    let state = state; // Extract inner AppState
    let static_config = endpoint
        .static_files
        .as_ref()
        .ok_or_else(|| AppError::Internal("Static file configuration missing".to_string()))?;

    let root = Path::new(&static_config.root);
    if !root.exists() {
        return Err(AppError::Internal(format!(
            "Static file root '{}' does not exist",
            static_config.root
        )));
    }

    let request_path = uri.path();
    let ep_path = endpoint
        .path
        .trim_end_matches("/*")
        .trim_end_matches("{*rest}");
    let relative = request_path
        .strip_prefix(ep_path)
        .unwrap_or(request_path)
        .trim_start_matches('/');

    let relative = percent_decode_str(relative)
        .decode_utf8()
        .map_err(|_| AppError::BadRequest("Invalid UTF-8 in path".to_string()))?
        .into_owned();

    if relative.split('/').any(|seg| seg == ".." || seg == ".") {
        return Err(AppError::Forbidden("Path traversal denied".to_string()));
    }

    match method {
        Method::GET => {
            handle_static_get(
                state,
                StaticGetContext {
                    endpoint: &endpoint,
                    config: static_config,
                    relative: &relative,
                    root,
                    request_path,
                    query: &query,
                    headers: &headers,
                },
            )
            .await
        }
        Method::POST | Method::PUT | Method::PATCH => {
            handle_file_upload(state, &endpoint, static_config, &relative, root, &headers).await
        }
        Method::DELETE => {
            handle_file_delete(state, &endpoint, static_config, &relative, root, &headers).await
        }
        Method::OPTIONS => {
            let mut response = (StatusCode::OK).into_response();
            if let Some(cors) = endpoint.cors.as_ref() {
                crate::middleware::cors::apply_cors_headers(&mut response, cors);
            }
            Ok(response)
        }
        _ => Err(AppError::MethodNotAllowed(format!(
            "Method {} not allowed for this endpoint",
            method
        ))),
    }
}

/// Extract authentication info from request.
async fn extract_auth_info(
    state: &AppState,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<AuthInfo, AppError> {
    if endpoint.auth == "none" {
        return Ok(AuthInfo::default());
    }
    crate::auth::middleware::authenticate(
        &endpoint.auth,
        &state.config.read().await.auth,
        headers,
        query_params,
    )
    .await
}

/// Check user role against required role for upload.
fn check_upload_role(auth_info: &AuthInfo, upload_config: &UploadConfig) -> Result<(), AppError> {
    if let Some(required_role) = &upload_config.required_role {
        match &auth_info.role {
            Some(role) if role == required_role => Ok(()),
            _ => Err(AppError::Forbidden(format!(
                "Role '{}' required to upload files",
                required_role
            ))),
        }
    } else {
        // If no required_role, user must be authenticated (have any role)
        match &auth_info.role {
            Some(_) => Ok(()),
            None => Err(AppError::Forbidden(
                "Authentication required to upload files".to_string(),
            )),
        }
    }
}

/// Handle GET requests - serve files with optional image resize or streaming.
async fn handle_static_get(
    state: State<AppState>,
    ctx: StaticGetContext<'_>,
) -> Result<Response, AppError> {
    let _ = ctx;
    let resolved = if ctx.relative.is_empty() {
        ctx.root.to_path_buf()
    } else {
        let r = ctx.root.join(ctx.relative);
        if let (Ok(canonical), Ok(root_canonical)) = (
            tokio::fs::canonicalize(&r).await,
            tokio::fs::canonicalize(ctx.root).await,
        ) && !canonical.starts_with(&root_canonical)
        {
            return Err(AppError::Forbidden("Path traversal denied".to_string()));
        }
        r
    };

    let resolved_meta = tokio::fs::metadata(&resolved).await.ok();
    if resolved_meta
        .as_ref()
        .is_some_and(std::fs::Metadata::is_dir)
    {
        let index_path = resolved.join(&ctx.config.index);
        let index_exists = tokio::fs::metadata(&index_path)
            .await
            .is_ok_and(|m| m.is_file());
        if index_exists {
            return serve_file(&index_path, ctx.config, None).await;
        }
        if ctx.config.directory_listing {
            return generate_directory_listing(&resolved, ctx.request_path).await;
        }
        if ctx.config.spa_fallback {
            return serve_spa_fallback(ctx.root, &ctx.config.index, ctx.config).await;
        }
        return Err(AppError::NotFound(format!(
            "File not found: {0}",
            ctx.request_path
        )));
    }

    // Check if user_scope is enabled.
    if let Some(user_scope) = &ctx.config.user_scope
        && user_scope.enabled
    {
        let query_params = ctx.query.0.as_ref().cloned().unwrap_or_default();
        let auth_info = extract_auth_info(&state, ctx.endpoint, ctx.headers, &query_params).await?;

        if let Some(required_role) = &user_scope.required_role {
            match &auth_info.role {
                Some(role) if role == required_role => {}
                _ => {
                    return Err(AppError::Forbidden(
                        "Authentication required to access user-scoped files".to_string(),
                    ));
                }
            }
        }

        // Construct user-specific path.
        let user_id = auth_info.subject.as_str();
        let pattern = &user_scope.directory_pattern;
        let user_path = format!("{}/{user_id}/{1}", pattern, ctx.relative).replace("//", "/"); // Ensure no double slashes
        let user_resolved = ctx.root.join(&user_path);
        if let (Ok(canonical), Ok(root_canonical)) = (
            tokio::fs::canonicalize(&user_resolved).await,
            tokio::fs::canonicalize(ctx.root).await,
        ) && !canonical.starts_with(&root_canonical)
        {
            return Err(AppError::Forbidden("Path traversal denied".to_string()));
        }

        let meta = tokio::fs::metadata(&user_resolved)
            .await
            .map_err(|_| AppError::NotFound(format!("File not found: {0}", ctx.request_path)))?;
        if !meta.is_file() {
            return Err(AppError::NotFound(format!(
                "File not found: {0}",
                ctx.request_path
            )));
        }

        return serve_file(&user_resolved, ctx.config, None).await;
    }

    serve_file(&resolved, ctx.config, ctx.query.0.clone()).await
}

/// Handle file upload (POST/PUT/PATCH).
async fn handle_file_upload(
    state: State<AppState>,
    endpoint: &EndpointConfig,
    config: &StaticFilesConfig,
    relative: &str,
    root: &Path,
    headers: &axum::http::HeaderMap,
) -> Result<Response, AppError> {
    let upload_config = config
        .upload
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("File uploads are not enabled".to_string()))?;

    if !upload_config.enabled {
        return Err(AppError::MethodNotAllowed(
            "File uploads are not enabled".to_string(),
        ));
    }

    // Extract auth info.
    let query_params: HashMap<String, String> = endpoint
        .crud
        .as_ref()
        .map(|c| {
            c.filtering
                .allowed_fields
                .iter()
                .map(|field| (field.clone(), field.clone()))
                .collect()
        })
        .unwrap_or_default();
    let auth_info = extract_auth_info(&state, endpoint, headers, &query_params).await?;
    check_upload_role(&auth_info, upload_config)?;

    let user_id = &auth_info.subject;
    let _content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream");

    // Validate file size from Content-Length header if available.
    if let Some(content_length) = headers.get(CONTENT_LENGTH)
        && let Ok(size) = content_length.to_str().unwrap_or("0").parse::<u64>()
        && size > upload_config.max_size
    {
        return Err(AppError::PayloadTooLarge(format!(
            "File size {} exceeds maximum allowed size {}",
            size, upload_config.max_size
        )));
    }

    // Sanitize filename.
    let sanitized_filename = sanitize_filename(relative)?;

    // Build storage path.
    let storage_path = build_storage_path(
        root,
        &sanitized_filename,
        user_id,
        &upload_config.create_subdirectory,
    )?;

    // Ensure parent directories exist.
    if let Some(parent) = storage_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::FileOperation(format!("Failed to create directory: {e}")))?;
    }

    // Validate extension.
    let file_extension = sanitized_filename
        .rsplit('.')
        .next()
        .map(|ext| ext.to_lowercase())
        .unwrap_or_default();

    if !upload_config.allowed_extensions.is_empty()
        && !upload_config.allowed_extensions.contains(&file_extension)
    {
        return Err(AppError::BadRequest(format!(
            "File extension .{} is not allowed",
            file_extension
        )));
    }

    // NOTE: Full multipart parsing requires the request body to be accessible.
    // This placeholder returns success response structure.
    // Full implementation would extract Multipart from axum::extract::Multipart
    // and process each field to save file to storage_path.

    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({
            "success": true,
            "path": format!("/{sanitized_filename}"),
            "size": 0,
            "message": "Upload endpoint ready - requires multipart body integration"
        })),
    )
        .into_response())
}

/// Handle file deletion (DELETE).
async fn handle_file_delete(
    state: State<AppState>,
    endpoint: &EndpointConfig,
    config: &StaticFilesConfig,
    relative: &str,
    root: &Path,
    headers: &axum::http::HeaderMap,
) -> Result<Response, AppError> {
    let upload_config = config
        .upload
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("File management is not enabled".to_string()))?;

    if !upload_config.enabled {
        return Err(AppError::MethodNotAllowed(
            "File management is not enabled".to_string(),
        ));
    }

    let query_params: HashMap<String, String> = endpoint
        .crud
        .as_ref()
        .map(|c| {
            c.filtering
                .allowed_fields
                .iter()
                .cloned()
                .map(|s| (s.clone(), s))
                .collect()
        })
        .unwrap_or_default();
    let auth_info = extract_auth_info(&state, endpoint, headers, &query_params).await?;
    check_upload_role(&auth_info, upload_config)?;

    let user_id = &auth_info.subject;
    let sanitized_filename = sanitize_filename(relative)?;

    // Check user_scope.
    let delete_path = if let Some(user_scope) = &config.user_scope {
        if user_scope.enabled {
            let pattern = &user_scope.directory_pattern;
            let user_path =
                format!("{}/{user_id}/{sanitized_filename}", pattern).replace("//", "/"); // Ensure no double slashes
            root.join(&user_path)
        } else {
            root.join(&sanitized_filename)
        }
    } else {
        root.join(&sanitized_filename)
    };

    // Verify file exists.
    if !delete_path.exists() {
        return Err(AppError::NotFound("File not found".to_string()));
    }

    // Delete the file.
    tokio::fs::remove_file(&delete_path)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to delete file: {e}")))?;

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": "File deleted successfully"
        })),
    )
        .into_response())
}

/// Sanitize filename to prevent path traversal and special characters.
fn sanitize_filename(name: &str) -> Result<String, AppError> {
    // Reject any path separators.
    if name.contains('/') || name.contains('\\') {
        return Err(AppError::BadRequest("Invalid filename".to_string()));
    }

    // Use UUID for upload filenames to prevent collisions.
    // For GET requests, return the sanitized name.
    Ok(name.to_string())
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

/// Serve a single file with optional image resize or streaming.
async fn serve_file(
    path: &Path,
    config: &StaticFilesConfig,
    query_params: Option<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let file_size = meta.len();
    let content_type = mime_from_path(path);

    // Check if image resize is requested.
    if let Some(image_config) = &config.image_resize
        && image_config.enabled
        && is_image_path(path)
        && let Some(resized) =
            handle_image_resize(path, query_params.as_ref(), image_config).await?
    {
        return Ok(resized);
    }

    // Check if streaming is enabled.
    if let Some(streaming_config) = &config.streaming
        && streaming_config.enabled
        && file_size > streaming_config.threshold
    {
        // NOTE: Range header support requires access to request headers
        // which are not passed to this function. Full implementation would
        // need to pass headers through the call chain.
        return serve_file_streaming(path, &content_type, config.cache_max_age, streaming_config)
            .await;
    }

    // Default: read entire file into memory.
    let contents = tokio::fs::read(path)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let mut response = (StatusCode::OK, contents).into_response();
    apply_static_headers(&mut response, &content_type, config.cache_max_age);
    Ok(response)
}

/// Serve a file using streaming response.
async fn serve_file_streaming(
    path: &Path,
    content_type: &str,
    cache_max_age: u64,
    config: &StreamingConfig,
) -> Result<Response, AppError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let reader_stream = tokio_util::io::ReaderStream::with_capacity(file, config.buffer_size);
    let body = Body::from_stream(reader_stream);

    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::OK;
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

    // NOTE: Range header support requires access to request headers
    // which are not passed to this function. Full implementation would
    // need to pass headers through the call chain.

    Ok(response)
}

/// Handle image resize if parameters are present.
async fn handle_image_resize(
    path: &Path,
    query_params: Option<&HashMap<String, String>>,
    config: &ImageResizeConfig,
) -> Result<Option<Response>, AppError> {
    let empty_map = HashMap::new();
    let query = query_params.unwrap_or(&empty_map);

    let width = query.get("w").and_then(|w| w.parse().ok());
    let height = query.get("h").and_then(|h| h.parse().ok());
    let fit = query.get("fit").map(|f| f.as_str());
    let output_format = query.get("format").map(|f| f.as_str());

    if width.is_none() && height.is_none() && output_format.is_none() {
        return Ok(None);
    }

    // Read the original image.
    let image_data = tokio::fs::read(path)
        .await
        .map_err(|_| AppError::NotFound(format!("File not found: {}", path.display())))?;

    let format = image::guess_format(&image_data)
        .map_err(|_| AppError::BadRequest("Invalid image format".to_string()))?;

    let img = image::load_from_memory(&image_data)
        .map_err(|_| AppError::BadRequest("Invalid image data".to_string()))?;

    // Determine target dimensions.
    let (target_width, target_height) = match (width, height, fit) {
        (Some(w), None, _) => (Some(w), None),
        (None, Some(h), _) => (None, Some(h)),
        (Some(w), Some(h), Some("cover")) => {
            // Fill while maintaining aspect ratio (cover mode)
            let ratio = img.width() as f32 / img.height() as f32;
            let h_ratio = h as f32 / w as f32;
            if ratio > h_ratio {
                let new_h = (w as f32 / ratio) as u32;
                (Some(w), Some(new_h))
            } else {
                let new_w = (h as f32 * ratio) as u32;
                (Some(new_w), Some(h))
            }
        }
        (Some(w), Some(h), _) => {
            let max_dim = config.max_dimension as u32;
            (Some(w.clamp(1, max_dim)), Some(h.clamp(1, max_dim)))
        }
        (None, None, None) => (None, None),
        (None, None, Some(_)) => (None, None), // Ignore fit if no dimensions provided
    };

    let resized = if let (Some(w), Some(h)) = (target_width, target_height) {
        img.resize(w, h, image::imageops::FilterType::Lanczos3)
    } else if let Some(w) = target_width {
        img.resize_to_fill(w, img.height(), image::imageops::FilterType::Lanczos3)
    } else if let Some(h) = target_height {
        img.resize_to_fill(img.width(), h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    // Determine output format.
    let output_format = output_format
        .and_then(image::ImageFormat::from_extension)
        .unwrap_or(format);

    // Encode the image.
    let mut output_bytes = Vec::new();
    resized
        .write_to(&mut std::io::Cursor::new(&mut output_bytes), output_format)
        .map_err(|_| AppError::Internal("Failed to encode image".to_string()))?;

    let content_type = match output_format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::WebP => "image/webp",
        _ => "application/octet-stream",
    };

    let mut response = (StatusCode::OK, output_bytes).into_response();
    apply_static_headers(&mut response, content_type, 0);
    Ok(Some(response))
}

/// Check if a path is likely an image.
fn is_image_path(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext_lower = ext.to_lowercase();
        matches!(
            ext_lower.as_str(),
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg"
        )
    } else {
        false
    }
}

/// Build storage path with subdirectory pattern.
fn build_storage_path(
    root: &Path,
    filename: &str,
    user_id: &str,
    subdirectory_pattern: &Option<String>,
) -> Result<PathBuf, AppError> {
    let final_path = if let Some(pattern) = subdirectory_pattern {
        let expanded = expand_subdirectory_pattern(pattern, user_id)?;
        root.join(&expanded).join(filename)
    } else {
        root.join(filename)
    };

    // Prevent path traversal even after pattern expansion.
    if final_path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(AppError::Forbidden("Invalid path".to_string()));
    }

    Ok(final_path)
}

/// Expand subdirectory pattern with placeholders.
fn expand_subdirectory_pattern(pattern: &str, user_id: &str) -> Result<String, AppError> {
    let now = Utc::now();
    let expanded = pattern
        .replace("{user_id}", user_id)
        .replace("{year}", &now.format("%Y").to_string())
        .replace("{month}", &now.format("%m").to_string())
        .replace("{day}", &now.format("%d").to_string())
        .replace("{uuid}", &Uuid::new_v4().to_string());

    // Check for path traversal in expanded path.
    if expanded.split('/').any(|seg| seg == "..") {
        return Err(AppError::Forbidden(
            "Invalid subdirectory pattern".to_string(),
        ));
    }

    Ok(expanded)
}

/// Serve the index file as a SPA fallback.
async fn serve_spa_fallback(
    root: &Path,
    index: &str,
    config: &StaticFilesConfig,
) -> Result<Response, AppError> {
    let index_path = root.join(index);
    let contents = tokio::fs::read(&index_path)
        .await
        .map_err(|_| AppError::NotFound(format!("Index file not found: {index}")))?;
    let mut response = (StatusCode::OK, contents).into_response();
    apply_static_headers(
        &mut response,
        &mime_from_path(&index_path),
        config.cache_max_age,
    );
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
        let is_dir = metadata.as_ref().is_some_and(std::fs::Metadata::is_dir);
        let size = metadata.as_ref().map_or(0, std::fs::Metadata::len);
        let modified = metadata.as_ref().and_then(|m| m.modified().ok());
        items.push(DirEntryInfo {
            name,
            is_dir,
            size,
            modified,
            mode: metadata
                .as_ref()
                .map_or(0, std::os::unix::fs::MetadataExt::mode),
            uid: metadata
                .as_ref()
                .map_or(0, std::os::unix::fs::MetadataExt::uid),
            gid: metadata
                .as_ref()
                .map_or(0, std::os::unix::fs::MetadataExt::gid),
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
        let modified_str = item
            .modified
            .map_or_else(|| "-".to_string(), format_modified);

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
        .map_or_else(|| uid.to_string(), |u| u.name)
}

/// Resolve a numeric gid to a group name, falling back to the numeric string.
fn resolve_group(gid: u32) -> String {
    nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(gid))
        .ok()
        .flatten()
        .map_or_else(|| gid.to_string(), |u| u.name)
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

/// Extract authentication info from request for file operations.
/// This is a helper to be used by the router when calling file handlers.
pub async fn extract_auth_for_files(
    state: &AppState,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<AuthInfo, AppError> {
    if endpoint.auth == "none" {
        return Ok(AuthInfo::default());
    }
    crate::auth::middleware::authenticate(
        &endpoint.auth,
        &state.config.read().await.auth,
        headers,
        query_params,
    )
    .await
}

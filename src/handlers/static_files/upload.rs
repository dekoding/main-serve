use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use http::header::CONTENT_LENGTH;

use crate::config::types::{EndpointConfig, StaticFilesConfig, UploadConfig, mime_from_path};
use crate::error::AppError;
use crate::handlers::common::helpers::is_image_extension;
use crate::handlers::common::path::{build_storage_path, generate_upload_filename};
use crate::handlers::common::utils::extract_auth_info;
use crate::server::state::AppState;
use crate::storage::{FileMetadata, Storage};

/// Context struct for file upload operations.
///
/// Bundles all data needed by the upload pipeline stages: validation,
/// processing, storage, and response building.
pub(crate) struct FileUploadContext {
    pub user_id: String,
    pub file_content: Vec<u8>,
    pub original_filename: String,
    pub storage_path: PathBuf,
    pub root: PathBuf,
    pub storage: Arc<dyn Storage>,
}

/// Handle file upload (POST/PUT/PATCH).
///
/// # Arguments
///
/// This function takes many parameters due to the complexity of multipart
/// file uploads with authentication, configuration, and storage handling.
#[allow(clippy::too_many_arguments)] // multipart upload handler with many axum extractors
pub async fn handle_file_upload(
    mut multipart: axum::extract::Multipart,
    state: State<AppState>,
    endpoint: &EndpointConfig,
    config: &StaticFilesConfig,
    _relative: &str,
    root: &Path,
    headers: &axum::http::HeaderMap,
    storage: Arc<dyn Storage>,
) -> Result<axum::http::Response<Body>, AppError> {
    let (ctx, upload_config) = validate_input(
        &mut multipart,
        &state,
        endpoint,
        config,
        root,
        headers,
        storage,
    )
    .await?;

    let sanitized_filename =
        generate_upload_filename(&ctx.original_filename, &upload_config.allowed_extensions)?;

    let storage_path = build_storage_path(
        &ctx.root,
        &sanitized_filename,
        &ctx.user_id,
        &upload_config.create_subdirectory,
    )?;

    let ctx = FileUploadContext {
        user_id: ctx.user_id,
        file_content: ctx.file_content,
        original_filename: ctx.original_filename,
        storage_path,
        root: ctx.root,
        storage: ctx.storage,
    };

    if ctx.storage.exists(&ctx.storage_path).await {
        return Err(AppError::FileOperation(format!(
            "A file already exists at the destination path: {}",
            ctx.storage_path.display()
        )));
    }

    store_file(&ctx).await?;

    build_response(&ctx, &sanitized_filename).await
}

/// Validate configuration, auth, and size limits before processing the upload.
///
/// Parses the multipart form, extracts auth info, validates file size and
/// image magic bytes, and returns the context ready for filename generation
/// and storage.
async fn validate_input(
    multipart: &mut axum::extract::Multipart,
    state: &State<AppState>,
    endpoint: &EndpointConfig,
    config: &StaticFilesConfig,
    root: &Path,
    headers: &axum::http::HeaderMap,
    storage: Arc<dyn Storage>,
) -> Result<(FileUploadContext, Arc<UploadConfig>), AppError> {
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
                .map(|field| (field.clone(), field.clone()))
                .collect()
        })
        .unwrap_or_default();
    let auth_info = extract_auth_info(state, endpoint, headers, &query_params).await?;
    let user_id = auth_info.subject;

    if let Some(content_length) = headers.get(CONTENT_LENGTH)
        && let Ok(size) = content_length.to_str().unwrap_or("0").parse::<u64>()
        && size > upload_config.max_size
    {
        return Err(AppError::PayloadTooLarge(format!(
            "File size {} exceeds maximum allowed size {}",
            size, upload_config.max_size
        )));
    }

    let (file_content, original_filename) =
        process_multipart(multipart, upload_config.max_size).await?;

    Ok((
        FileUploadContext {
            user_id,
            file_content,
            original_filename,
            storage_path: PathBuf::new(),
            root: root.to_path_buf(),
            storage,
        },
        Arc::new(upload_config.clone()),
    ))
}

/// Parse the multipart form and extract file content.
///
/// Reads the first field from the multipart stream, validates its metadata,
/// accumulates all chunks, and validates the content size. For image files,
/// magic bytes are validated.
///
/// # Errors
///
/// Returns `AppError::BadRequest` for parse, size, or magic-byte failures.
async fn process_multipart(
    multipart: &mut axum::extract::Multipart,
    max_size: u64,
) -> Result<(Vec<u8>, String), AppError> {
    let mut file_content = Vec::new();
    let mut original_filename = String::new();
    let mut found_file = false;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to parse multipart form: {e}")))?
    {
        if let Some(field_name) = field.name() {
            if field_name != "file" && field_name != "files" && field_name != "upload" {
                // Allow other field names but log a warning
            }
        } else {
            return Err(AppError::BadRequest("Form field missing name".to_string()));
        }

        if let Some(filename) = field.file_name() {
            original_filename = filename.to_string();
        }

        found_file = true;
        let mut field_bytes = Vec::new();
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| AppError::BadRequest(format!("Failed to read file content: {e}")))?
        {
            field_bytes.extend_from_slice(&chunk);
        }
        file_content = field_bytes;

        if file_content.len() as u64 > max_size {
            return Err(AppError::PayloadTooLarge(format!(
                "File size {} exceeds maximum allowed size {}",
                file_content.len(),
                max_size
            )));
        }

        if let Some(ext) = std::path::Path::new(&original_filename)
            .extension()
            .and_then(|e| e.to_str())
            && is_image_extension(ext)
        {
            validate_image_magic_bytes(&file_content)?;
        }
    }

    if !found_file {
        return Err(AppError::BadRequest(
            "No file provided in multipart form".to_string(),
        ));
    }

    Ok((file_content, original_filename))
}

/// Store the file in the configured storage backend.
///
/// Creates parent directories as needed, writes the file content, and
/// retrieves metadata for the response builder.
///
/// # Errors
///
/// Returns `AppError::FileOperation` if directory creation or file writing fails.
async fn store_file(ctx: &FileUploadContext) -> Result<FileMetadata, AppError> {
    let storage_path = &ctx.storage_path;

    if let Some(parent) = storage_path.parent() {
        ctx.storage
            .create_dir_all(parent)
            .await
            .map_err(|e| AppError::FileOperation(format!("Failed to create directory: {e}")))?;
    }

    ctx.storage
        .write(storage_path, &ctx.file_content)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to write file: {e}")))?;

    ctx.storage
        .metadata(storage_path)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to read file metadata: {e}")))
}

/// Build the HTTP response for a successful file upload.
///
/// Returns a 201 CREATED response with JSON body containing file metadata.
async fn build_response(
    ctx: &FileUploadContext,
    sanitized_filename: &str,
) -> Result<axum::http::Response<Body>, AppError> {
    let metadata = ctx
        .storage
        .metadata(&ctx.storage_path)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to read file metadata: {e}")))?;

    let created = metadata.created.map_or_else(
        || chrono::Utc::now().to_rfc3339(),
        |t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339(),
    );

    let modified = metadata
        .modified
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
        .unwrap_or(created.clone());

    let mime_type = mime_from_path(&ctx.storage_path);

    let relative_path = ctx.storage_path.strip_prefix(&ctx.root).map_or_else(
        |_| format!("/{sanitized_filename}"),
        |p| format!("/{}", p.to_string_lossy()),
    );

    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({
            "success": true,
            "path": relative_path,
            "name": sanitized_filename,
            "size": ctx.file_content.len(),
            "type": mime_type,
            "created": created,
            "modified": modified,
            "message": "File uploaded successfully"
        })),
    )
        .into_response())
}

/// Validate magic bytes for image files.
///
/// This provides an extra layer of security by checking the actual file
/// format rather than relying solely on file extension.
fn validate_image_magic_bytes(data: &[u8]) -> Result<(), AppError> {
    // PNG signature
    if data.len() >= 8 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
        return Ok(());
    }

    // JPEG signature: must be 0xFF 0xD8 0xFF (SOI + start of marker)
    if data.len() >= 3 && &data[0..3] == b"\xFF\xD8\xFF" {
        return Ok(());
    }

    // GIF signature
    if data.len() >= 6 && (&data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a") {
        return Ok(());
    }

    // BMP signature
    if data.len() >= 2 && &data[0..2] == b"BM" {
        return Ok(());
    }

    // WebP signature
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Ok(());
    }

    // SVG signature (XML-based)
    if data.len() >= 4 && data[0] == b'<' && data[1] == b'?' {
        return Ok(());
    }

    Err(AppError::BadRequest(
        "File does not appear to be a valid image based on magic bytes".to_string(),
    ))
}

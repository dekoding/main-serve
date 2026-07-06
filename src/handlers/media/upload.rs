/// Media upload handler.
use std::path::PathBuf;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use crate::config::types::mime_from_path;
use crate::db::query::builders::build_insert;
use crate::db::query::types::MutationContext;
use crate::error::AppError;
use crate::handlers::common::helpers::parse_multipart_file;
use crate::handlers::common::path::{build_storage_path, sanitize_filename};
use crate::handlers::common::utils::HandlerContext;
use crate::middleware::auth::extractor::RequestContext;

/// Handle media upload.
pub async fn handle_media_upload(
    handler_ctx: &HandlerContext<'_>,
    mut multipart: axum::extract::Multipart,
    _uri: &axum::http::Uri,
) -> Result<Response, AppError> {
    let config = &handler_ctx
        .endpoint
        .media
        .as_ref()
        .ok_or_else(|| AppError::NotFound("Media config not found".to_string()))?;

    let upload_config = config
        .upload
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Media upload is not enabled".to_string()))?;

    if upload_config.max_size == 0 {
        return Err(AppError::MethodNotAllowed(
            "Media upload is not enabled".to_string(),
        ));
    }

    let storage = handler_ctx
        .state
        .get_store(&config.storage)
        .ok_or_else(|| AppError::Internal(format!("Store '{}' not found", config.storage)))?;

    let root = storage
        .root_path()
        .unwrap_or_else(|| PathBuf::from(&config.storage));

    let auth_info = handler_ctx.extract_auth_info().await?;
    let user_id = &auth_info.subject;

    let (file_content, original_filename) =
        parse_multipart_file(&mut multipart, upload_config.max_size, false).await?;

    if file_content.len() as u64 > upload_config.max_size {
        return Err(AppError::PayloadTooLarge(format!(
            "File size {} exceeds maximum allowed size {}",
            file_content.len(),
            upload_config.max_size
        )));
    }

    let sanitized_filename = format!(
        "{}-{}",
        Uuid::new_v4(),
        sanitize_filename(&original_filename, false)?
    );

    let storage_path = build_storage_path(
        &root,
        &sanitized_filename,
        user_id,
        &upload_config.create_subdirectory,
    )?;

    if let Some(parent) = storage_path.parent() {
        storage
            .create_dir_all(parent)
            .await
            .map_err(|e| AppError::FileOperation(format!("Failed to create directory: {e}")))?;
    }

    let file_extension = sanitized_filename
        .rsplit('.')
        .next()
        .map(|ext: &str| ext.to_lowercase())
        .unwrap_or_default();

    if !upload_config.allowed_extensions.is_empty() {
        let normalized: Vec<String> = upload_config
            .allowed_extensions
            .iter()
            .map(|ext| ext.trim_start_matches('.').to_lowercase())
            .collect();
        if !normalized.contains(&file_extension) {
            return Err(AppError::BadRequest(format!(
                "File extension .{file_extension} is not allowed"
            )));
        }
    }

    storage
        .write(&storage_path, &file_content)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to write file: {e}")))?;

    storage
        .metadata(&storage_path)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to read file metadata: {e}")))?;

    let mime_type = mime_from_path(&storage_path);

    // Insert media record into the database
    let db_ctx = handler_ctx
        .state
        .get_db_context(&config.database, &config.table)
        .await?;

    let file_path = storage_path.strip_prefix(&root).map_or_else(
        |_| sanitized_filename.clone(),
        |p| p.to_string_lossy().to_string(),
    );

    let now = chrono::Utc::now().to_rfc3339();
    let mut insert_map = serde_json::Map::new();
    insert_map.insert(
        "original_name".to_string(),
        serde_json::Value::String(sanitized_filename.clone()),
    );
    insert_map.insert(
        "mime_type".to_string(),
        serde_json::Value::String(mime_type.to_string()),
    );
    insert_map.insert(
        "size".to_string(),
        serde_json::Value::Number(serde_json::Number::from(file_content.len())),
    );
    insert_map.insert(
        "uploader_id".to_string(),
        serde_json::Value::String(user_id.to_string()),
    );
    insert_map.insert(
        "file_path".to_string(),
        serde_json::Value::String(file_path.clone()),
    );
    insert_map.insert(
        "created_at".to_string(),
        serde_json::Value::String(now.clone()),
    );
    insert_map.insert(
        "updated_at".to_string(),
        serde_json::Value::String(now.clone()),
    );

    let writable_fields = db_ctx
        .table_config
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect();

    let crud_config = crate::config::types::CrudConfig {
        writable_fields,
        ..Default::default()
    };

    let ctx = MutationContext::from(&crud_config);

    let built = build_insert(
        &db_ctx,
        &ctx,
        &serde_json::Value::Object(insert_map),
        &RequestContext::default(),
    )?;

    let returned_id = if built.sql.contains("RETURNING") {
        let row = db_ctx
            .pool
            .fetch_optional_json(&built.sql, &built.params)
            .await?;
        row.and_then(|r| r.get("id").cloned())
            .and_then(|v| v.as_str().map(String::from))
    } else {
        db_ctx
            .pool
            .execute_with_params(&built.sql, &built.params)
            .await?;
        None
    };

    let relative_path = format!("/{file_path}");

    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({
            "success": true,
            "path": relative_path,
            "name": sanitized_filename,
            "size": file_content.len(),
            "type": mime_type,
            "created": now,
            "message": "Media uploaded successfully",
            "id": returned_id,
        })),
    )
        .into_response())
}

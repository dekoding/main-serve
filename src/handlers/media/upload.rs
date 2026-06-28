/// Media upload handler.
use std::collections::HashMap;
use std::path::PathBuf;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use crate::config::types::{DatabaseDriver, EndpointConfig, MediaConfig, TableConfig};
use crate::db::query::builders::build_insert;
use crate::error::AppError;
use crate::handlers::media::extract_auth_info;
use crate::handlers::static_files::upload::{build_storage_path, sanitize_filename};
use crate::handlers::static_files::utils::mime_from_path;
use crate::middleware::auth::extractor::RequestContext;

/// Handle media upload.
pub async fn handle_media_upload(
    state: axum::extract::State<crate::server::state::AppState>,
    mut multipart: axum::extract::Multipart,
    endpoint: &EndpointConfig,
    _uri: &axum::http::Uri,
    headers: &axum::http::HeaderMap,
    query_params: HashMap<String, String>,
) -> Result<Response, AppError> {
    let config = endpoint
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

    let storage = state
        .get_store(&config.storage)
        .ok_or_else(|| AppError::Internal(format!("Store '{}' not found", config.storage)))?;

    let root = storage
        .root_path()
        .unwrap_or(PathBuf::from(&config.storage));

    let auth_info = extract_auth_info(&state, endpoint, headers, &query_params).await?;
    let user_id = &auth_info.subject;

    let mut file_content = Vec::new();
    let mut original_filename = String::new();
    let mut found_file = false;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to parse multipart form: {e}")))?
    {
        if let Some(filename) = field.file_name() {
            original_filename = filename.to_string();
        }

        let mut field_bytes = Vec::new();
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| AppError::BadRequest(format!("Failed to read file content: {e}")))?
        {
            field_bytes.extend_from_slice(&chunk);
        }

        file_content = field_bytes;
        found_file = true;
    }

    if !found_file || file_content.is_empty() {
        return Err(AppError::BadRequest(
            "No file provided in multipart form".to_string(),
        ));
    }

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
    let (pool, table_config, driver) = get_db_context(&state, config).await?;

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

    let writable_fields = table_config
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect();

    let crud_config = crate::config::types::CrudConfig {
        writable_fields,
        ..Default::default()
    };

    let built = build_insert(
        &table_config,
        &crud_config,
        &serde_json::Value::Object(insert_map),
        driver,
        &RequestContext::default(),
    )?;

    let returned_id = if built.sql.contains("RETURNING") {
        let row = pool.fetch_optional_json(&built.sql, &built.params).await?;
        row.and_then(|r| r.get("id").cloned())
            .and_then(|v| v.as_str().map(String::from))
    } else {
        pool.execute_with_params(&built.sql, &built.params).await?;
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

async fn get_db_context(
    state: &crate::server::state::AppState,
    config: &MediaConfig,
) -> Result<(crate::db::pool::DatabasePool, TableConfig, DatabaseDriver), AppError> {
    let pool = {
        let pools = state.db_pools.read().await;
        pools
            .get(config.database.as_str())
            .ok_or_else(|| {
                AppError::Internal(format!("Database '{}' has no pool", config.database))
            })?
            .clone()
    };
    let driver = pool.driver();

    let table_config = {
        let config_guard = state.config.read().await;
        config_guard
            .tables
            .iter()
            .find(|t| t.name == config.table && t.database == config.database)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "Table '{}' in database '{}' not found in config",
                    config.table, config.database
                ))
            })?
            .clone()
    };

    Ok((pool, table_config, driver))
}

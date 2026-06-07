/// Media library handler.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use http::HeaderValue;
use http::header;
use uuid::Uuid;

use crate::config::types::{DatabaseDriver, EndpointConfig, MediaConfig};
use crate::db::query::builders::{
    build_delete, build_insert, build_select_list, build_select_one, build_update,
};
use crate::db::query::types::QueryParams;
use crate::error::AppError;
use crate::handlers::static_files::upload::build_storage_path;
use crate::handlers::static_files::utils::mime_from_path;
use crate::middleware::auth::extractor::{AuthInfo, RequestContext};
use crate::server::state::AppState;
use crate::storage::Storage;

/// Route handler for media library endpoints.
pub async fn handle_media_route(
    state: axum::extract::State<AppState>,
    matched_path: axum::extract::MatchedPath,
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
    query: axum::extract::Query<HashMap<String, String>>,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config_for_method(path_str, &method)
        .await
        .or_else(|| {
            use crate::server::prefix_match::{find_prefix_match, find_wildcard_match};
            let configs = state.endpoint_configs.blocking_read();
            find_prefix_match(&configs, path_str, |_| true)
                .or_else(|| find_wildcard_match(&configs, path_str, |_| true))
        })
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    let body_value = if body.is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
    };

    handle_media(
        &state,
        method,
        uri,
        &endpoint,
        headers,
        query.0,
        &body_value,
    )
    .await
}

/// Route handler for media upload (multipart POST).
pub async fn handle_media_upload_route(
    state: axum::extract::State<AppState>,
    matched_path: axum::extract::MatchedPath,
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
    query: axum::extract::Query<HashMap<String, String>>,
    multipart: axum::extract::Multipart,
) -> Result<Response, AppError> {
    let path_str = matched_path.as_str();

    let endpoint = state
        .get_endpoint_config_for_method(path_str, &method)
        .await
        .or_else(|| {
            use crate::server::prefix_match::{find_prefix_match, find_wildcard_match};
            let configs = state.endpoint_configs.blocking_read();
            find_prefix_match(&configs, path_str, |_| true)
                .or_else(|| find_wildcard_match(&configs, path_str, |_| true))
        })
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    if method != axum::http::Method::POST
        && method != axum::http::Method::PUT
        && method != axum::http::Method::PATCH
    {
        return Err(AppError::MethodNotAllowed(
            "Method not allowed for media upload".to_string(),
        ));
    }

    let config = endpoint
        .media
        .as_ref()
        .ok_or_else(|| AppError::NotFound("Media config not found".to_string()))?;

    if config.upload.as_ref().is_some_and(|u| u.max_size == 0) {
        return Err(AppError::MethodNotAllowed(
            "Media upload is not enabled".to_string(),
        ));
    }

    handle_media_upload(state, multipart, &endpoint, &uri, &headers, query.0).await
}

/// Core media handler logic.
#[allow(clippy::collapsible_if)]
pub async fn handle_media(
    state: &AppState,
    method: axum::http::Method,
    uri: axum::http::Uri,
    endpoint: &EndpointConfig,
    headers: axum::http::HeaderMap,
    query_params: HashMap<String, String>,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let config = endpoint
        .media
        .as_ref()
        .ok_or_else(|| AppError::NotFound("Media config not found".to_string()))?;

    let path = uri.path().to_string();

    let storage = state
        .get_store(&config.storage)
        .ok_or_else(|| AppError::Internal(format!("Store '{}' not found", config.storage)))?;

    let root = storage
        .root_path()
        .unwrap_or(PathBuf::from(&config.storage));
    let (pool, table_config, driver) = get_db_context(state, config).await?;

    // Dispatch based on path patterns
    if path.starts_with("/_main-serve/media/trash") {
        return handle_media_trash(
            state,
            method,
            &path,
            config,
            &*storage,
            &root,
            &pool,
            &table_config,
            driver,
            endpoint,
            &headers,
            &query_params,
        )
        .await;
    }

    if path.starts_with("/shared/") {
        let token = path.strip_prefix("/shared/").unwrap_or("");
        return handle_media_share_get(token, config, &*storage, &root).await;
    }

    if path.contains("/resize") {
        if let Some(id) = extract_media_id(&path) {
            return handle_media_resize(
                &*storage,
                &root,
                &id,
                config,
                &query_params,
                &pool,
                &table_config,
            )
            .await;
        }
    }

    if path.contains("/thumbnail") {
        if let Some(id) = extract_media_id(&path) {
            return handle_media_thumbnail(&*storage, &root, &id, config, &pool, &table_config)
                .await;
        }
    }

    if path.contains("/attach") {
        if let Some(id) = extract_media_id(&path) {
            return handle_media_attach(&id, config, &pool, body).await;
        }
    }

    if path.contains("/detach") {
        if let Some(id) = extract_media_id(&path) {
            return handle_media_detach(&id, config, &pool, body).await;
        }
    }

    if path.contains("/move") {
        if let Some(id) = extract_media_id(&path) {
            return handle_media_move(&id, config, &pool, &*storage, &root, body).await;
        }
    }

    if path.contains("/rename") {
        if let Some(id) = extract_media_id(&path) {
            return handle_media_rename(&id, config, &pool, &*storage, &root, body).await;
        }
    }

    match method {
        axum::http::Method::GET => {
            if path.is_empty() || path == "/" || path == config.table {
                handle_media_list(&pool, config, &table_config, driver, &query_params).await
            } else {
                match extract_media_id(&path) {
                    Some(id) => handle_media_get(&pool, config, &table_config, driver, &id).await,
                    None => Err(AppError::BadRequest("Media ID required".to_string())),
                }
            }
        }
        axum::http::Method::POST => {
            if path.is_empty() || path == "/" {
                handle_media_create(
                    &pool,
                    config,
                    &table_config,
                    driver,
                    endpoint,
                    &headers,
                    body,
                    state,
                    &query_params,
                )
                .await
            } else {
                Err(AppError::MethodNotAllowed(
                    "POST not allowed on this path".to_string(),
                ))
            }
        }
        axum::http::Method::PATCH => match extract_media_id(&path) {
            Some(id) => {
                handle_media_update(
                    state,
                    &id,
                    config,
                    &table_config,
                    driver,
                    endpoint,
                    &headers,
                    body,
                    &query_params,
                )
                .await
            }
            None => Err(AppError::BadRequest("Media ID required".to_string())),
        },
        axum::http::Method::DELETE => match extract_media_id(&path) {
            Some(id) => {
                handle_media_delete(
                    &table_config,
                    state,
                    &id,
                    config,
                    &*storage,
                    &root,
                    &pool,
                    driver,
                    endpoint,
                    &headers,
                    &query_params,
                )
                .await
            }
            None => Err(AppError::BadRequest("Media ID required".to_string())),
        },
        _ => Err(AppError::MethodNotAllowed(
            "Method not allowed for media endpoint".to_string(),
        )),
    }
}

/// Handle media list (GET /).
async fn handle_media_list(
    pool: &crate::db::pool::DatabasePool,
    config: &MediaConfig,
    table_config: &crate::config::types::TableConfig,
    driver: DatabaseDriver,
    query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    let page = query_params
        .get("page")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(config.pagination.default_page_size);

    let page_size = query_params
        .get("page_size")
        .or_else(|| query_params.get("per_page"))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(config.pagination.default_page_size);

    let sort = query_params.get("sort").cloned().or_else(|| {
        if !config.sorting.default_field.is_empty() {
            Some(config.sorting.default_field.clone())
        } else {
            None
        }
    });

    let order = Some(config.sorting.default_order);

    let reserved = ["page", "page_size", "per_page", "sort", "order"];
    let filters: HashMap<String, String> = query_params
        .iter()
        .filter(|(k, _)| !reserved.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let qp = QueryParams {
        page: Some(page),
        page_size: Some(page_size),
        sort,
        order,
        filters,
    };

    let built = build_select_list(
        &config.table,
        table_config,
        &crate::config::types::CrudConfig::default(),
        &qp,
        driver,
        &RequestContext::default(),
    )?;
    let rows = pool.fetch_all_json(&built.sql, &built.params).await?;

    let count_built = build_select_list_count(&config.table, table_config, driver, &qp)?;
    let count_row = pool
        .fetch_optional_json(&count_built.sql, &count_built.params)
        .await?;
    let total: i64 = count_row
        .as_ref()
        .and_then(|r| r.get("count"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let response = serde_json::json!({
        "data": rows,
        "pagination": {
            "page": page,
            "page_size": page_size,
            "total": total,
            "total_pages": (total as f64 / page_size as f64).ceil() as u64,
        }
    });

    Ok((StatusCode::OK, axum::Json(response)).into_response())
}

fn build_select_list_count(
    table_name: &str,
    _table_config: &crate::config::types::TableConfig,
    driver: DatabaseDriver,
    query_params: &QueryParams,
) -> Result<crate::db::query::types::BuiltQuery, AppError> {
    let base_sql = format!("SELECT COUNT(*) as count FROM {}", table_name);
    let mut where_clauses: Vec<String> = Vec::new();
    let mut params: Vec<serde_json::Value> = Vec::new();
    let mut param_idx = 1usize;

    for (key, value) in &query_params.filters {
        if ["page", "page_size", "per_page", "sort", "order"].contains(&key.as_str()) {
            continue;
        }
        where_clauses.push(format!(
            "{} = {}",
            key,
            crate::db::query::helpers::placeholder(driver, param_idx)
        ));
        params.push(serde_json::Value::String(value.clone()));
        param_idx += 1;
    }

    let sql = if where_clauses.is_empty() {
        base_sql
    } else {
        format!("{} WHERE {}", base_sql, where_clauses.join(" AND "))
    };

    Ok(crate::db::query::types::BuiltQuery { sql, params })
}

/// Handle media get by ID.
async fn handle_media_get(
    pool: &crate::db::pool::DatabasePool,
    config: &MediaConfig,
    table_config: &crate::config::types::TableConfig,
    driver: DatabaseDriver,
    id: &str,
) -> Result<Response, AppError> {
    let built = build_select_one(
        &config.table,
        table_config,
        &crate::config::types::CrudConfig::default(),
        id,
        driver,
        &RequestContext::default(),
    )?;
    match pool.fetch_optional_json(&built.sql, &built.params).await? {
        Some(row) => Ok((StatusCode::OK, axum::Json(row)).into_response()),
        None => Err(AppError::NotFound(format!(
            "Media item with id '{}' not found",
            &id
        ))),
    }
}

/// Handle media create (POST /).
#[allow(clippy::too_many_arguments)]
async fn handle_media_create(
    pool: &crate::db::pool::DatabasePool,
    config: &MediaConfig,
    table_config: &crate::config::types::TableConfig,
    driver: DatabaseDriver,
    endpoint: &EndpointConfig,
    __headers: &axum::http::HeaderMap,
    body: &serde_json::Value,
    state: &AppState,
    query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    let user_id = extract_user_id(state, endpoint, __headers, query_params).await?;

    let writable_columns = table_config
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect::<Vec<_>>();
    let mut body_map = serde_json::Map::new();

    if let Some(obj) = body.as_object() {
        for (k, v) in obj {
            if writable_columns.contains(&k.to_string()) {
                body_map.insert(k.clone(), v.clone());
            }
        }
    }

    if writable_columns.contains(&"uploader_id".to_string()) {
        body_map.insert(
            "uploader_id".to_string(),
            serde_json::Value::String(user_id),
        );
    }

    if writable_columns.contains(&"created_at".to_string()) {
        body_map.insert(
            "created_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
    }

    let json_body = serde_json::Value::Object(body_map);
    let built = build_insert(
        &config.table,
        table_config,
        &crate::config::types::CrudConfig::default(),
        &json_body,
        driver,
        &RequestContext::default(),
    )?;

    if built.sql.contains("RETURNING") {
        let row = pool.fetch_optional_json(&built.sql, &built.params).await?;
        Ok((
            StatusCode::CREATED,
            axum::Json(serde_json::json!({ "data": row })),
        )
            .into_response())
    } else {
        let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;
        Ok((
            StatusCode::CREATED,
            axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
        )
            .into_response())
    }
}

/// Handle media update (PATCH /:id).
#[allow(clippy::collapsible_if)]
#[allow(clippy::too_many_arguments)]
async fn handle_media_update(
    state: &AppState,
    id: &str,
    config: &MediaConfig,
    table_config: &crate::config::types::TableConfig,
    driver: DatabaseDriver,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    body: &serde_json::Value,
    query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    if let Some(user_scope) = &config.user_scope {
        if user_scope.enabled && !user_scope.allow_cross_user_browse {
            let auth_info = extract_auth_info(state, endpoint, headers, query_params).await?;
            let current_user = auth_info.subject;

            let pool = {
                let pools = state.db_pools.read().await;
                pools
                    .get(&config.database)
                    .ok_or_else(|| {
                        AppError::Internal(format!("Database '{}' has no pool", config.database))
                    })?
                    .clone()
            };

            let user_check = format!("SELECT uploader_id FROM {} WHERE id = $1", config.table);
            let row = pool.fetch_optional_json(&user_check, &[id.into()]).await?;
            if let Some(row) = row {
                if let Some(uploader_id) = row.get("uploader_id").and_then(|v| v.as_str()) {
                    if uploader_id != current_user {
                        return Err(AppError::Forbidden(
                            "Cannot update media owned by another user".to_string(),
                        ));
                    }
                }
            }
        }
    }

    let pool = {
        let pools = state.db_pools.read().await;
        pools
            .get(&config.database)
            .ok_or_else(|| {
                AppError::Internal(format!("Database '{}' has no pool", config.database))
            })?
            .clone()
    };

    let auth_info = extract_auth_info(state, endpoint, headers, query_params).await?;
    let mut body_map = serde_json::Map::new();

    if let Some(obj) = body.as_object() {
        for (k, v) in obj {
            if table_config.columns.iter().any(|c| c.name == *k) {
                body_map.insert(k.clone(), v.clone());
            }
        }
    }

    if table_config.columns.iter().any(|c| c.name == "updated_at") {
        body_map.insert(
            "updated_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
    }
    body_map.insert(
        "last_modified_by".to_string(),
        serde_json::Value::String(auth_info.subject),
    );

    let built = build_update(
        &config.table,
        table_config,
        &crate::config::types::CrudConfig::default(),
        id,
        &serde_json::Value::Object(body_map),
        driver,
        &RequestContext::default(),
    )?;
    let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "Media item with id '{}' not found",
            &id
        )));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
    )
        .into_response())
}

/// Handle media upload.
async fn handle_media_upload(
    state: axum::extract::State<AppState>,
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

    if !upload_config.allowed_extensions.is_empty()
        && !upload_config
            .allowed_extensions
            .iter()
            .any(|ext| ext == &file_extension)
    {
        return Err(AppError::BadRequest(format!(
            "File extension .{file_extension} is not allowed"
        )));
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
    let relative_path = storage_path.strip_prefix(&root).map_or_else(
        |_| format!("/{sanitized_filename}"),
        |p| format!("/{}", p.to_string_lossy()),
    );

    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({
            "success": true,
            "path": relative_path,
            "name": sanitized_filename,
            "size": file_content.len(),
            "type": mime_type,
            "created": chrono::Utc::now().to_rfc3339(),
            "message": "Media uploaded successfully"
        })),
    )
        .into_response())
}

/// Handle media resize.
async fn handle_media_resize(
    storage: &dyn Storage,
    root: &Path,
    id: &str,
    config: &MediaConfig,
    query_params: &HashMap<String, String>,
    pool: &crate::db::pool::DatabasePool,
    _table_config: &crate::config::types::TableConfig,
) -> Result<Response, AppError> {
    let image_resize = config
        .image_resize
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Image resize is not enabled".to_string()))?;

    if !image_resize.enabled {
        return Err(AppError::MethodNotAllowed(
            "Image resize is not enabled".to_string(),
        ));
    }

    // Look up file_path from DB
    let sql = format!("SELECT file_path FROM {} WHERE id = $1", config.table);
    let row = pool.fetch_optional_json(&sql, &[id.into()]).await?;

    let file_path: String = row
        .and_then(|r| {
            r.get("file_path")
                .and_then(|v| v.as_str().map(String::from))
        })
        .unwrap_or_else(|| format!("media/{}", id));

    let resolved_path = root.join(&file_path);

    let image_data = storage
        .read(&resolved_path)
        .await
        .map_err(|_| AppError::NotFound(format!("Media file not found: {}", file_path)))?;

    let format = image::ImageFormat::from_extension("jpg").unwrap_or(image::ImageFormat::Png);
    let img = image::load_from_memory(&image_data)
        .map_err(|_| AppError::BadRequest("Invalid image data".to_string()))?;

    let width = query_params.get("w").and_then(|w| w.parse().ok());
    let height = query_params.get("h").and_then(|h| h.parse().ok());
    let fit = query_params.get("fit").map(std::string::String::as_str);

    let (target_width, target_height) = match (width, height, fit) {
        (Some(w), None, _) => (Some(w), None),
        (None, Some(h), _) => (None, Some(h)),
        (Some(w), Some(h), Some("cover")) => {
            let ratio = img.width() as f64 / img.height() as f64;
            let h_ratio = h as f64 / w as f64;
            if ratio > h_ratio {
                let new_h = (w as f64 / ratio) as u32;
                (Some(w), Some(new_h))
            } else {
                let new_w = (h as f64 * ratio) as u32;
                (Some(new_w), Some(h))
            }
        }
        (Some(w), Some(h), _) => {
            let max_dim = u32::try_from(image_resize.max_dimension).unwrap_or(u32::MAX);
            (Some(w.clamp(1, max_dim)), Some(h.clamp(1, max_dim)))
        }
        _ => (Some(img.width()), Some(img.height())),
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

    let output_format = query_params
        .get("format")
        .and_then(image::ImageFormat::from_extension)
        .unwrap_or(format);

    let mut output_bytes = Vec::new();
    resized
        .write_to(&mut std::io::Cursor::new(&mut output_bytes), output_format)
        .map_err(|_| AppError::Internal("Failed to encode resized image".to_string()))?;

    let content_type = match output_format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::Gif => "image/gif",
        image::ImageFormat::WebP => "image/webp",
        _ => "application/octet-stream",
    };

    let mut response = (StatusCode::OK, output_bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str("public, max-age=86400")
            .unwrap_or(HeaderValue::from_static("public, max-age=3600")),
    );

    Ok(response)
}

async fn handle_media_thumbnail(
    storage: &dyn Storage,
    root: &Path,
    id: &str,
    config: &MediaConfig,
    pool: &crate::db::pool::DatabasePool,
    table_config: &crate::config::types::TableConfig,
) -> Result<Response, AppError> {
    let image_resize = config
        .image_resize
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Image resize is not enabled".to_string()))?;

    let default_size = image_resize
        .styles
        .iter()
        .find(|s| s.name == "thumbnail")
        .map(|s| s.max_width.min(s.max_height))
        .unwrap_or(150);

    let mut params = HashMap::new();
    params.insert("w".to_string(), default_size.to_string());
    params.insert("h".to_string(), default_size.to_string());

    handle_media_resize(storage, root, id, config, &params, pool, table_config).await
}

/// Handle trash management routes.
#[allow(clippy::too_many_arguments)]
async fn handle_media_trash(
    state: &AppState,
    method: axum::http::Method,
    path: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    pool: &crate::db::pool::DatabasePool,
    table_config: &crate::config::types::TableConfig,
    driver: DatabaseDriver,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    let trash_config = config
        .trash
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Trash is not enabled".to_string()))?;

    if !trash_config.enabled {
        return Err(AppError::MethodNotAllowed(
            "Trash is not enabled".to_string(),
        ));
    }

    match method {
        axum::http::Method::GET => {
            handle_media_trash_list(pool, config, table_config, driver).await
        }
        axum::http::Method::DELETE
            if path == "/_main-serve/media/trash" || path == "/_main-serve/media/trash/" =>
        {
            handle_media_trash_empty(config, pool, table_config, driver).await
        }
        axum::http::Method::POST => {
            if let Some(id) = path
                .strip_prefix("/_main-serve/media/trash/")
                .and_then(|p| p.strip_suffix("/restore"))
            {
                handle_media_trash_restore(
                    state,
                    id,
                    config,
                    storage,
                    root,
                    pool,
                    driver,
                    endpoint,
                    headers,
                    query_params,
                )
                .await
            } else {
                Err(AppError::BadRequest(
                    "Invalid trash restore path".to_string(),
                ))
            }
        }
        axum::http::Method::DELETE => {
            if let Some(id) = path.strip_prefix("/_main-serve/media/trash/") {
                handle_media_trash_permanent_delete(
                    state,
                    id,
                    config,
                    storage,
                    root,
                    pool,
                    table_config,
                    driver,
                    endpoint,
                    headers,
                    query_params,
                )
                .await
            } else {
                Err(AppError::BadRequest(
                    "Invalid trash delete path".to_string(),
                ))
            }
        }
        _ => Err(AppError::MethodNotAllowed(
            "Method not allowed for trash endpoint".to_string(),
        )),
    }
}

async fn handle_media_trash_list(
    pool: &crate::db::pool::DatabasePool,
    config: &MediaConfig,
    table_config: &crate::config::types::TableConfig,
    driver: DatabaseDriver,
) -> Result<Response, AppError> {
    let qp = QueryParams {
        page: None,
        page_size: None,
        sort: None,
        order: Some(crate::config::types::SortOrder::Desc),
        filters: HashMap::new(),
    };

    let built = build_select_list(
        &config.table,
        table_config,
        &crate::config::types::CrudConfig::default(),
        &qp,
        driver,
        &RequestContext::default(),
    )?;
    let sql = format!("{} WHERE trashed_at IS NOT NULL", built.sql);
    let rows = pool.fetch_all_json(&sql, &[]).await?;

    Ok((StatusCode::OK, axum::Json(rows)).into_response())
}

#[allow(clippy::too_many_arguments)]
async fn handle_media_trash_restore(
    state: &AppState,
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    pool: &crate::db::pool::DatabasePool,
    _driver: DatabaseDriver,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    let trash_config = config
        .trash
        .as_ref()
        .ok_or_else(|| AppError::Internal("Trash not enabled".to_string()))?;

    let sql = format!(
        "SELECT id, file_path FROM {} WHERE id = $1 AND trashed_at IS NOT NULL",
        config.table
    );
    let row = pool.fetch_optional_json(&sql, &[id.into()]).await?;

    let file_path = match row {
        Some(r) => r
            .get("file_path")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default(),
        None => {
            return Err(AppError::NotFound(
                "Trashed media item not found".to_string(),
            ));
        }
    };

    let auth_info = extract_auth_info(state, endpoint, headers, query_params).await?;
    let user_path = auth_info.subject;

    let trash_path = root
        .join(&trash_config.prefix)
        .join(&user_path)
        .join(&file_path);
    let restore_path = root.join(file_path);

    if storage.exists(&trash_path).await {
        if let Some(parent) = restore_path.parent() {
            storage
                .create_dir_all(parent)
                .await
                .map_err(|e| AppError::FileOperation(format!("Failed to create directory: {e}")))?;
        }
        storage
            .rename(&trash_path, &restore_path)
            .await
            .map_err(|e| AppError::FileOperation(format!("Failed to restore file: {e}")))?;
    }

    let update_sql = format!(
        "UPDATE {} SET trashed_at = NULL, deleted_at = NULL WHERE id = $1",
        config.table
    );
    pool.execute_with_params(&update_sql, &[id.into()]).await?;

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": "Media item restored from trash"
        })),
    )
        .into_response())
}

#[allow(clippy::too_many_arguments)]
async fn handle_media_trash_empty(
    config: &MediaConfig,
    pool: &crate::db::pool::DatabasePool,
    table_config: &crate::config::types::TableConfig,
    driver: DatabaseDriver,
) -> Result<Response, AppError> {
    let sql = format!(
        "SELECT id FROM {} WHERE trashed_at IS NOT NULL",
        config.table
    );
    let rows = pool.fetch_all_json(&sql, &[]).await?;

    let mut deleted_count = 0u64;

    for row in &rows {
        if let Some(id_val) = row.get("id").and_then(|v| v.as_str()) {
            let built = build_delete(&config.table, table_config, id_val, driver)?;
            let _ = pool.execute_with_params(&built.sql, &built.params).await;
            deleted_count += 1;
        }
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": format!("Emptied trash, deleted {} items", deleted_count),
            "deleted_count": deleted_count,
        })),
    )
        .into_response())
}

#[allow(clippy::too_many_arguments)]
async fn handle_media_trash_permanent_delete(
    state: &AppState,
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    pool: &crate::db::pool::DatabasePool,
    table_config: &crate::config::types::TableConfig,
    driver: DatabaseDriver,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    let trash_config = config
        .trash
        .as_ref()
        .ok_or_else(|| AppError::Internal("Trash not enabled".to_string()))?;

    let sql = format!(
        "SELECT id, file_path FROM {} WHERE id = $1 AND trashed_at IS NOT NULL",
        config.table
    );
    let row = pool.fetch_optional_json(&sql, &[id.into()]).await?;

    let file_path: String = if let Some(r) = row {
        r.get("file_path")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default()
    } else {
        String::new()
    };

    if !file_path.is_empty() {
        let auth_info = extract_auth_info(state, endpoint, headers, query_params).await?;
        let trash_path = root
            .join(&trash_config.prefix)
            .join(&auth_info.subject)
            .join(&file_path);
        if storage.exists(&trash_path).await {
            let _ = storage.delete(&trash_path).await;
        }
    }

    let built = build_delete(&config.table, table_config, id, driver)?;
    let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(
            "Trashed media item not found".to_string(),
        ));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": "Media item permanently deleted from trash",
            "rows_affected": rows_affected,
        })),
    )
        .into_response())
}

/// Handle sharing: GET /shared/:token.
async fn handle_media_share_get(
    token: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
) -> Result<Response, AppError> {
    let sharing_config = config
        .sharing
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Sharing is not enabled".to_string()))?;

    let shared_path = root.join(&sharing_config.prefix).join(token);

    match storage.metadata(&shared_path).await {
        Ok(meta) => {
            let content_type = mime_from_path(&shared_path);
            let contents = storage
                .read(&shared_path)
                .await
                .map_err(|_| AppError::NotFound("Shared file not found".to_string()))?;

            let mut response = (StatusCode::OK, contents).into_response();
            let headers = response.headers_mut();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(&content_type)
                    .unwrap_or(HeaderValue::from_static("application/octet-stream")),
            );
            headers.insert(
                header::CONTENT_LENGTH,
                HeaderValue::from_str(&meta.size.to_string())
                    .unwrap_or(HeaderValue::from_static("0")),
            );
            Ok(response)
        }
        Err(_) => Err(AppError::NotFound(
            "Shared file not found or share link expired".to_string(),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_media_delete(
    table_config: &crate::config::types::TableConfig,
    state: &AppState,
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    pool: &crate::db::pool::DatabasePool,
    driver: DatabaseDriver,
    endpoint: &EndpointConfig,
    __headers: &axum::http::HeaderMap,
    _query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    let trash_enabled = config.trash.as_ref().is_some_and(|t| t.enabled);

    if trash_enabled {
        handle_media_trash_delete(
            table_config,
            state,
            id,
            config,
            storage,
            root,
            pool,
            driver,
            endpoint,
            __headers,
            _query_params,
        )
        .await
    } else {
        delete_media_permanently(id, config, storage, root, pool, table_config, driver).await
    }
}

#[allow(clippy::collapsible_if)]
async fn delete_media_permanently(
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    pool: &crate::db::pool::DatabasePool,
    table_config: &crate::config::types::TableConfig,
    driver: DatabaseDriver,
) -> Result<Response, AppError> {
    let path_check = format!("SELECT file_path FROM {} WHERE id = $1", config.table);
    let row = pool.fetch_optional_json(&path_check, &[id.into()]).await?;

    if let Some(row) = row {
        if let Some(file_path) = row.get("file_path").and_then(|v| v.as_str()) {
            let file_path_buf = root.join(file_path);
            if storage.exists(&file_path_buf).await {
                storage
                    .delete(&file_path_buf)
                    .await
                    .map_err(|e| AppError::FileOperation(format!("Failed to delete file: {e}")))?;
            }
        }
    }

    let built = build_delete(&config.table, table_config, id, driver)?;
    let rows_affected = pool.execute_with_params(&built.sql, &built.params).await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "Media item with id '{}' not found",
            &id
        )));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({ "rows_affected": rows_affected })),
    )
        .into_response())
}

#[allow(clippy::too_many_arguments)]
async fn handle_media_trash_delete(
    _table_config: &crate::config::types::TableConfig,
    state: &AppState,
    id: &str,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
    pool: &crate::db::pool::DatabasePool,
    _driver: DatabaseDriver,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<Response, AppError> {
    let trash_config = config
        .trash
        .as_ref()
        .ok_or_else(|| AppError::Internal("Trash not enabled but delete was called".to_string()))?;

    let auth_info = extract_auth_info(state, endpoint, headers, query_params).await?;
    let user_id = auth_info.subject;

    let path_check = format!("SELECT file_path FROM {} WHERE id = $1", config.table);
    let row = pool.fetch_optional_json(&path_check, &[id.into()]).await?;

    let file_path = row
        .as_ref()
        .and_then(|r| {
            r.get("file_path")
                .and_then(|v| v.as_str().map(String::from))
        })
        .unwrap_or_default();

    let user_path = user_id.clone();

    let trash_prefix = &trash_config.prefix;
    let trash_dest = root.join(trash_prefix).join(&user_path).join(&file_path);

    if let Some(parent) = trash_dest.parent() {
        storage.create_dir_all(parent).await.map_err(|e| {
            AppError::FileOperation(format!("Failed to create trash directory: {e}"))
        })?;
    }

    let source_path = root.join(file_path);
    if storage.exists(&source_path).await {
        storage
            .rename(&source_path, &trash_dest)
            .await
            .map_err(|e| AppError::FileOperation(format!("Failed to move file to trash: {e}")))?;
    }

    let update_sql = format!(
        "UPDATE {} SET deleted_at = NOW(), trashed_at = NOW() WHERE id = $1",
        config.table
    );
    let rows_affected = pool.execute_with_params(&update_sql, &[id.into()]).await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "Media item with id '{}' not found",
            &id
        )));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": "Media moved to trash",
            "rows_affected": rows_affected,
        })),
    )
        .into_response())
}

/// Handle content reference attach.
async fn handle_media_attach(
    id: &str,
    config: &MediaConfig,
    pool: &crate::db::pool::DatabasePool,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let content_refs = config
        .content_references
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Content references not enabled".to_string()))?;

    if !content_refs.enabled {
        return Err(AppError::MethodNotAllowed(
            "Content references are disabled".to_string(),
        ));
    }

    let entity_id = body
        .get("entity_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("entity_id is required".to_string()))?
        .to_string();

    let content_type = body
        .get("content_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("content_type is required".to_string()))?;

    let entity_id_str = entity_id.to_string();
    let content_type_str = content_type.to_string();

    if let Some(allowed) = &content_refs.allowed_content_types
        && !allowed.is_empty()
        && !allowed.contains(&content_type_str)
    {
        return Err(AppError::BadRequest(format!(
            "content_type '{}' is not allowed. Allowed: {:?}",
            content_type, allowed
        )));
    }

    let table = &content_refs.table;
    let media_id_col = &content_refs.media_id_column;
    let entity_id_col = &content_refs.entity_id_column;
    let content_type_col = &content_refs.content_type_column;
    let order_col = &content_refs.order_column;

    let max_order_sql = format!(
        "SELECT COALESCE(MAX({}), 0) as max_order FROM {}",
        order_col, table
    );
    let max_row = pool.fetch_optional_json(&max_order_sql, &[]).await?;
    let max_order: i64 = max_row
        .as_ref()
        .and_then(|r| r.get("max_order").and_then(|v| v.as_i64()))
        .unwrap_or(0);

    let next_order = max_order + 1;

    let insert_sql = format!(
        "INSERT INTO {} ({}, {}, {}, {}) VALUES ($1, $2, $3, $4)",
        table, media_id_col, entity_id_col, content_type_col, order_col
    );

    pool.execute_with_params(
        &insert_sql,
        &[
            id.into(),
            serde_json::Value::String(entity_id_str),
            serde_json::Value::String(content_type_str.clone()),
            serde_json::Value::Number(serde_json::Number::from(next_order)),
        ],
    )
    .await?;

    let select_sql = format!(
        "SELECT * FROM {} WHERE {} = $1 AND {} = $2 AND {} = $3",
        table, media_id_col, entity_id_col, content_type_col
    );
    let row = pool
        .fetch_optional_json(
            &select_sql,
            &[id.into(), entity_id.into(), content_type_str.into()],
        )
        .await?;

    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({ "data": row })),
    )
        .into_response())
}

/// Handle content reference detach.
async fn handle_media_detach(
    id: &str,
    config: &MediaConfig,
    pool: &crate::db::pool::DatabasePool,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let content_refs = config
        .content_references
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Content references not enabled".to_string()))?;

    if !content_refs.enabled {
        return Err(AppError::MethodNotAllowed(
            "Content references are disabled".to_string(),
        ));
    }

    let entity_id = body
        .get("entity_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("entity_id is required".to_string()))?;

    let content_type = body
        .get("content_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("content_type is required".to_string()))?;

    let table = &content_refs.table;
    let media_id_col = &content_refs.media_id_column;
    let entity_id_col = &content_refs.entity_id_column;
    let content_type_col = &content_refs.content_type_column;

    let delete_sql = format!(
        "DELETE FROM {} WHERE {} = $1 AND {} = $2 AND {} = $3",
        table, media_id_col, entity_id_col, content_type_col
    );

    let rows_affected = pool
        .execute_with_params(
            &delete_sql,
            &[
                id.into(),
                serde_json::Value::String(entity_id.to_string()),
                serde_json::Value::String(content_type.to_string()),
            ],
        )
        .await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "No content reference found for media id '{}', entity '{}', type '{}'",
            id, entity_id, content_type
        )));
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": format!(
                "Detached media '{}' from entity '{}:{}'",
                id, entity_id, content_type
            ),
            "rows_affected": rows_affected,
        })),
    )
        .into_response())
}

/// Handle media move (POST /:id/move).
async fn handle_media_move(
    id: &str,
    config: &MediaConfig,
    pool: &crate::db::pool::DatabasePool,
    storage: &dyn Storage,
    root: &Path,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let move_config = config
        .move_config
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Media move is not configured".to_string()))?;

    if !move_config.enabled {
        return Err(AppError::MethodNotAllowed(
            "Media move is disabled".to_string(),
        ));
    }

    let destination_path = body
        .get("destination_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("destination_path is required".to_string()))?;

    let sql = format!("SELECT file_path FROM {} WHERE id = $1", config.table);
    let row = pool.fetch_optional_json(&sql, &[id.into()]).await?;

    let current_file_path = row
        .and_then(|r| {
            r.get("file_path")
                .and_then(|v| v.as_str().map(String::from))
        })
        .ok_or_else(|| AppError::NotFound(format!("Media item with id '{}' not found", id)))?;

    let current_path = root.join(&current_file_path);

    let new_path = if destination_path.starts_with('/') {
        root.join(destination_path.trim_start_matches('/'))
    } else {
        root.join(destination_path)
    };

    if move_config.auto_create_destination
        && let Some(parent) = new_path.parent()
    {
        storage
            .create_dir_all(parent)
            .await
            .map_err(|e| AppError::FileOperation(format!("Failed to create directory: {e}")))?;
    }

    storage
        .rename(&current_path, &new_path)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to move file: {e}")))?;

    let new_relative = new_path
        .strip_prefix(root)
        .map(|p| format!("/{}", p.to_string_lossy()))
        .unwrap_or_else(|_| format!("/{}", new_path.to_string_lossy()));

    let update_sql = format!("UPDATE {} SET file_path = $1 WHERE id = $2", config.table);
    pool.execute_with_params(
        &update_sql,
        &[
            serde_json::Value::String(new_relative.trim_start_matches('/').to_string()),
            id.into(),
        ],
    )
    .await?;

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": "Media file moved successfully",
            "new_path": new_relative,
        })),
    )
        .into_response())
}

/// Handle media rename (PATCH /:id/rename).
async fn handle_media_rename(
    id: &str,
    config: &MediaConfig,
    pool: &crate::db::pool::DatabasePool,
    storage: &dyn Storage,
    root: &Path,
    body: &serde_json::Value,
) -> Result<Response, AppError> {
    let rename_config = config
        .rename
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed("Media rename is not configured".to_string()))?;

    if !rename_config.enabled {
        return Err(AppError::MethodNotAllowed(
            "Media rename is disabled".to_string(),
        ));
    }

    let new_name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("name is required".to_string()))?;

    let sql = format!("SELECT file_path FROM {} WHERE id = $1", config.table);
    let row = pool.fetch_optional_json(&sql, &[id.into()]).await?;

    let current_file_path = row
        .and_then(|r| {
            r.get("file_path")
                .and_then(|v| v.as_str().map(String::from))
        })
        .ok_or_else(|| AppError::NotFound(format!("Media item with id '{}' not found", id)))?;

    let current_path = root.join(&current_file_path);

    let parent = current_path
        .parent()
        .ok_or_else(|| AppError::BadRequest("Cannot determine parent directory".to_string()))?;

    let new_path = parent.join(new_name);

    storage
        .rename(&current_path, &new_path)
        .await
        .map_err(|e| AppError::FileOperation(format!("Failed to rename file: {e}")))?;

    let new_relative = new_path
        .strip_prefix(root)
        .map(|p| format!("/{}", p.to_string_lossy()))
        .unwrap_or_else(|_| format!("/{}", new_path.to_string_lossy()));

    let update_sql = format!("UPDATE {} SET file_path = $1 WHERE id = $2", config.table);
    pool.execute_with_params(
        &update_sql,
        &[
            serde_json::Value::String(new_relative.trim_start_matches('/').to_string()),
            id.into(),
        ],
    )
    .await?;

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "message": "Media file renamed successfully",
            "new_path": new_relative,
        })),
    )
        .into_response())
}

fn extract_media_id(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    segments.last().map(|s| s.to_string())
}

async fn extract_user_id(
    state: &AppState,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<String, AppError> {
    let auth_info = extract_auth_info(state, endpoint, headers, query_params).await?;
    Ok(auth_info.subject)
}

async fn extract_auth_info(
    state: &AppState,
    endpoint: &EndpointConfig,
    headers: &axum::http::HeaderMap,
    query_params: &HashMap<String, String>,
) -> Result<AuthInfo, AppError> {
    if endpoint.auth == "none" {
        return Ok(AuthInfo::default());
    }
    let auth_config = state.config.read().await.auth.clone();
    crate::middleware::auth::validate::authenticate(
        &endpoint.auth,
        &auth_config,
        headers,
        query_params,
    )
    .await
}

fn sanitize_filename(name: &str, _is_upload: bool) -> Result<String, AppError> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(AppError::BadRequest("Invalid filename".to_string()));
    }
    let sanitized: String = name
        .chars()
        .filter(|c| !c.is_ascii_control() && *c != '\0')
        .collect();
    if sanitized.is_empty() {
        return Err(AppError::BadRequest("Filename cannot be empty".to_string()));
    }
    Ok(sanitized.to_lowercase())
}

async fn get_db_context(
    state: &AppState,
    config: &MediaConfig,
) -> Result<
    (
        crate::db::pool::DatabasePool,
        crate::config::types::TableConfig,
        DatabaseDriver,
    ),
    AppError,
> {
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

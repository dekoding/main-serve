/// Media sharing handlers.
use std::path::Path;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use http::HeaderValue;
use http::header;

use crate::config::types::{MediaConfig, mime_from_path};
use crate::error::AppError;
use crate::storage::Storage;

/// Handle sharing: GET /shared/:token.
pub async fn handle_media_share_get(
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
                HeaderValue::from_str(content_type)
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

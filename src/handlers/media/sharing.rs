/// Media sharing handlers.
use std::path::Path;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::types::{EndpointConfig, MediaConfig, mime_from_path};
use crate::error::AppError;
use crate::handlers::common::utils::{apply_content_length, apply_content_type};
use crate::storage::Storage;

/// Handle sharing: GET /shared/:token.
///
/// # Errors
///
/// Returns an `AppError::MethodNotAllowed` if sharing is not enabled.
pub async fn handle_media_share_get(
    token: &str,
    endpoint: &EndpointConfig,
    config: &MediaConfig,
    storage: &dyn Storage,
    root: &Path,
) -> Result<Response, AppError> {
    let sharing_config = config
        .sharing
        .as_ref()
        .ok_or_else(|| AppError::MethodNotAllowed {
            message: "Sharing is not enabled".to_string(),
            allowed: endpoint.methods.clone(),
        })?;

    let shared_path = root.join(&sharing_config.prefix).join(token);

    match storage.metadata(&shared_path).await {
        Ok(meta) => {
            let content_type = mime_from_path(&shared_path);
            let contents = storage
                .read(&shared_path)
                .await
                .map_err(|_| AppError::NotFound("Shared file not found".to_string()))?;

            let mut response = (StatusCode::OK, contents).into_response();
            apply_content_type(&mut response, content_type);
            apply_content_length(&mut response, &meta.size.to_string());
            Ok(response)
        }
        Err(_) => Err(AppError::NotFound(
            "Shared file not found or share link expired".to_string(),
        )),
    }
}

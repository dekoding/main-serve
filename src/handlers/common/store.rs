/// Shared storage resolution helpers.
///
/// Eliminates duplicated store resolution logic across media, `file_store`,
/// `spa_host`, and `static_files` handlers.
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::AppError;
use crate::server::state::AppState;
use crate::storage::Storage;

/// Resolve a named storage store to an [`Arc<dyn Storage>`] and root [`PathBuf`].
///
/// For native stores, resolves the actual filesystem path.
/// For cloud stores, returns the store and a conceptual root path.
///
/// # Errors
///
/// Returns an `AppError::Internal` if the store name is not found.
pub fn resolve_store(
    state: &AppState,
    store_name: &str,
) -> Result<(Arc<dyn Storage>, PathBuf), AppError> {
    let storage = state
        .get_store(store_name)
        .ok_or_else(|| AppError::Internal(format!("Store '{store_name}' not found")))?;

    let root = storage
        .root_path()
        .unwrap_or_else(|| PathBuf::from(store_name));

    Ok((storage, root))
}

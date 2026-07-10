/// Shared storage resolution helpers.
///
/// Eliminates duplicated store resolution logic across media, file_store,
/// spa_host, and static_files handlers.
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::AppError;
use crate::server::state::AppState;
use crate::storage::Storage;

/// Resolve a named storage store to an Arc<dyn Storage> and root PathBuf.
///
/// For native stores, resolves the actual filesystem path.
/// For cloud stores, returns the store and a conceptual root path.
pub fn resolve_store(
    state: &AppState,
    store_name: &str,
) -> Result<(Arc<dyn Storage>, PathBuf), AppError> {
    let storage = state
        .get_store(store_name)
        .ok_or_else(|| AppError::Internal(format!("Store '{store_name}' not found")))?;

    let root = if let Some(path) = storage.root_path() {
        path
    } else {
        // For cloud stores, use the store name as a conceptual root
        PathBuf::from(store_name)
    };

    Ok((storage, root))
}

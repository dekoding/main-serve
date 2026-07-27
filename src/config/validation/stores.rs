use std::collections::HashMap;

use crate::config::types::{StoreBackend, StoreConfig};

/// Validate store configurations have required fields per backend type.
///
/// Checks that each store has the correct backend-specific config present,
/// all required fields within that config are non-empty, and no conflicting
/// backend configs are set simultaneously.
pub(crate) fn validate_stores(stores: &HashMap<String, StoreConfig>, errors: &mut Vec<String>) {
    for (name, store) in stores {
        let label = format!("stores.{name}");

        // Count non-None backend-specific config sections.
        let mut config_count: u32 = 0;
        if store.root.is_some() {
            config_count += 1;
        }
        if store.s3.is_some() {
            config_count += 1;
        }
        if store.azure.is_some() {
            config_count += 1;
        }
        if store.gcs.is_some() {
            config_count += 1;
        }

        // Spec rule: at most one backend-specific config section.
        if config_count > 1 {
            let mut present = Vec::new();
            if store.root.is_some() {
                present.push("root");
            }
            if store.s3.is_some() {
                present.push("s3");
            }
            if store.azure.is_some() {
                present.push("azure");
            }
            if store.gcs.is_some() {
                present.push("gcs");
            }
            errors.push(format!(
                "{label}: conflicting backend config sections: {present:?} (at most one allowed)"
            ));
        }

        match store.backend {
            StoreBackend::Native => {
                if store.root.as_deref().is_none_or(str::is_empty) {
                    errors.push(format!(
                        "{label}: root directory must not be empty for native backend"
                    ));
                }
            }
            StoreBackend::Memory => {
                // Memory backend requires no additional configuration.
                if store.root.as_deref().is_some_and(|s| !s.is_empty()) {
                    errors.push(format!("{label}: root must not be set for memory backend"));
                }
            }
            StoreBackend::S3 => {
                let Some(s3) = store.s3.as_ref() else {
                    errors.push(format!(
                        "{label}: s3 config section must be present for s3 backend"
                    ));
                    continue;
                };
                if s3.region.is_empty() {
                    errors.push(format!("{label}: s3.region must not be empty"));
                }
                if s3.bucket.is_empty() {
                    errors.push(format!("{label}: s3.bucket must not be empty"));
                }
                if s3.access_key.is_empty() {
                    errors.push(format!("{label}: s3.access_key must not be empty"));
                }
                if s3.secret_key.is_empty() {
                    errors.push(format!("{label}: s3.secret_key must not be empty"));
                }
            }
            StoreBackend::Azure => {
                let Some(azure) = store.azure.as_ref() else {
                    errors.push(format!(
                        "{label}: azure config section must be present for azure backend"
                    ));
                    continue;
                };
                if azure.account_name.is_empty() {
                    errors.push(format!("{label}: azure.account_name must not be empty"));
                }
                if azure.account_key.is_empty() {
                    errors.push(format!("{label}: azure.account_key must not be empty"));
                }
                if azure.container.is_empty() {
                    errors.push(format!("{label}: azure.container must not be empty"));
                }
            }
            StoreBackend::Gcs => {
                let Some(gcs) = store.gcs.as_ref() else {
                    errors.push(format!(
                        "{label}: gcs config section must be present for gcs backend"
                    ));
                    continue;
                };
                if gcs.project_id.is_empty() {
                    errors.push(format!("{label}: gcs.project_id must not be empty"));
                }
                if gcs.credentials.is_empty() {
                    errors.push(format!("{label}: gcs.credentials must not be empty"));
                }
                if gcs.bucket.is_empty() {
                    errors.push(format!("{label}: gcs.bucket must not be empty"));
                }
            }
        }
    }
}

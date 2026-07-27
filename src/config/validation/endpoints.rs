use crate::config::types::{AppConfig, EndpointAction};
use crate::db::query::helpers::is_safe_sql_fragment;

/// Validate endpoint configs: correct action types, valid references, etc.
pub fn validate_endpoints(config: &AppConfig, errors: &mut Vec<String>) {
    const HTTP_STATUS_MIN: u16 = 100;
    const HTTP_STATUS_MAX: u16 = 599;

    for (i, ep) in config.endpoints.iter().enumerate() {
        let label = format!("endpoints[{}] ({})", i, ep.path);

        if ep.path.is_empty() {
            errors.push(format!("{label}: path must not be empty"));
        }
        if ep.methods.is_empty() {
            errors.push(format!("{label}: must have at least one HTTP method"));
        }

        match ep.action {
            EndpointAction::Crud => {
                if let Some(ref crud) = ep.crud {
                    if crud.table.is_empty() {
                        errors.push(format!("{label}: crud.table must not be empty"));
                    } else if !config.tables.iter().any(|t| t.name == crud.table) {
                        errors.push(format!(
                            "{label}: crud.table '{}' is not defined in tables",
                            crud.table
                        ));
                    }
                    if crud.database.is_empty() {
                        errors.push(format!("{label}: crud.database must not be empty"));
                    } else if !config.databases.contains_key(&crud.database) {
                        errors.push(format!(
                            "{label}: crud.database '{}' is not defined in databases",
                            crud.database
                        ));
                    }
                    // Validate SQL fragments embedded in queries.
                    if let Some(ref wc) = crud.where_clause
                        && !is_safe_sql_fragment(wc)
                    {
                        errors.push(format!(
                            "{label}: crud.where_clause contains unsafe SQL characters (;, --, /*)"
                        ));
                    }
                    for (ji, join) in crud.joins.iter().enumerate() {
                        if !is_safe_sql_fragment(&join.on) {
                            errors.push(format!(
                                "{label}: crud.joins[{ji}].on contains unsafe SQL characters"
                            ));
                        }
                        if !is_safe_sql_fragment(&join.table) {
                            errors.push(format!(
                                "{label}: crud.joins[{ji}].table contains unsafe SQL characters"
                            ));
                        }
                    }
                    for (ci, cf) in crud.computed_fields.iter().enumerate() {
                        if !is_safe_sql_fragment(&cf.expression) {
                            errors.push(format!(
                                "{label}: crud.computed_fields[{ci}].expression contains unsafe SQL characters"
                            ));
                        }
                        if !is_safe_sql_fragment(&cf.name) {
                            errors.push(format!(
                                "{label}: crud.computed_fields[{ci}].name contains unsafe SQL characters"
                            ));
                        }
                    }
                } else {
                    errors.push(format!(
                        "{label}: action is 'crud' but no crud config provided"
                    ));
                }
            }
            EndpointAction::Proxy => {
                if let Some(ref proxy) = ep.proxy {
                    if proxy.upstream.is_empty() {
                        errors.push(format!("{label}: proxy.upstream must not be empty"));
                    }
                } else {
                    errors.push(format!(
                        "{label}: action is 'proxy' but no proxy config provided"
                    ));
                }
            }
            EndpointAction::StaticFiles => {
                if let Some(ref sf) = ep.static_files {
                    if sf.storage.is_empty() {
                        errors.push(format!("{label}: static_files.storage must not be empty"));
                    } else if !config.stores.contains_key(&sf.storage) {
                        errors.push(format!(
                            "{label}: static_files.storage references store '{}' which is not defined in stores",
                            sf.storage
                        ));
                    }
                } else {
                    errors.push(format!(
                        "{label}: action is 'static_files' but no static_files config provided"
                    ));
                }
            }
            EndpointAction::SpaHost => {
                if let Some(ref spa) = ep.spa_host {
                    if spa.storage.is_empty() {
                        errors.push(format!("{label}: spa_host.storage must not be empty"));
                    } else if !config.stores.contains_key(&spa.storage) {
                        errors.push(format!(
                            "{label}: spa_host.storage references store '{}' which is not defined in stores",
                            spa.storage
                        ));
                    }
                } else {
                    errors.push(format!(
                        "{label}: action is 'spa_host' but no spa_host config provided"
                    ));
                }
            }
            EndpointAction::Media => {
                if let Some(ref media) = ep.media {
                    if media.storage.is_empty() {
                        errors.push(format!("{label}: media.storage must not be empty"));
                    } else if !config.stores.contains_key(&media.storage) {
                        errors.push(format!(
                            "{label}: media.storage references store '{}' which is not defined in stores",
                            media.storage
                        ));
                    }
                    if media.table.is_empty() {
                        errors.push(format!("{label}: media.table must not be empty"));
                    } else if !config.tables.iter().any(|t| t.name == media.table) {
                        errors.push(format!(
                            "{label}: media.table '{}' is not defined in tables",
                            media.table
                        ));
                    }
                    if media.database.is_empty() {
                        errors.push(format!("{label}: media.database must not be empty"));
                    } else if !config.databases.contains_key(&media.database) {
                        errors.push(format!(
                            "{label}: media.database references database '{}' which is not defined in databases",
                            media.database
                        ));
                    }
                } else {
                    errors.push(format!(
                        "{label}: action is 'media' but no media config provided"
                    ));
                }
            }
            EndpointAction::FileStore => {
                if let Some(ref fs) = ep.file_store {
                    if fs.storage.is_empty() {
                        errors.push(format!("{label}: file_store.storage must not be empty"));
                    } else if !config.stores.contains_key(&fs.storage) {
                        errors.push(format!(
                            "{label}: file_store.storage references store '{}' which is not defined in stores",
                            fs.storage
                        ));
                    }
                    if fs.table.is_empty() {
                        errors.push(format!("{label}: file_store.table must not be empty"));
                    } else if !config.tables.iter().any(|t| t.name == fs.table) {
                        errors.push(format!(
                            "{label}: file_store.table '{}' is not defined in tables",
                            fs.table
                        ));
                    }
                    if fs.database.is_empty() {
                        errors.push(format!("{label}: file_store.database must not be empty"));
                    } else if !config.databases.contains_key(&fs.database) {
                        errors.push(format!(
                            "{label}: file_store.database references database '{}' which is not defined in databases",
                            fs.database
                        ));
                    }
                } else {
                    errors.push(format!(
                        "{label}: action is 'file_store' but no file_store config provided"
                    ));
                }
            }
            EndpointAction::CustomResponse => {
                if ep.custom_response.is_none() {
                    errors.push(format!(
                        "{label}: action is 'custom_response' but no custom_response config provided"
                    ));
                } else if let Some(cr) = ep.custom_response.as_ref()
                    && (cr.status < HTTP_STATUS_MIN || cr.status > HTTP_STATUS_MAX)
                {
                    errors.push(format!(
                        "{label}: custom_response.status must be a valid HTTP status code ({HTTP_STATUS_MIN}-{HTTP_STATUS_MAX})"
                    ));
                }
            }
        }
    }
}

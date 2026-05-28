/// YAML loading, parsing, and environment variable interpolation.
///
/// Supports `${ENV_VAR}` and `${ENV_VAR:-default}` syntax in any string value.
/// Environment variables are resolved at load time before YAML parsing.
///
/// Supports `$include` directives to split configuration across multiple files:
/// - In a sequence: `- $include: "path/to/file.yaml"` includes a single item
/// - In a sequence with glob: `- $include: "endpoints/*.yaml"` includes all matches
/// - In a mapping: `$include: "path/or/glob"` merges included mappings into the parent
///
/// Include paths are relative to the directory of the file containing the directive.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use std::sync::LazyLock;

use regex::Regex;
use serde_yaml::Value;

use super::types::AppConfig;
use super::validation::validate_config;
use crate::error::AppError;

/// Pre-compiled regex for env-var interpolation. Matches `${VAR}` and `${VAR:-default}`.
static ENV_VAR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-((?:[^}])*))?\}").expect("env var regex is valid")
});

/// Load, interpolate, parse, resolve includes, and validate a configuration file.
///
/// 1. Read the YAML file from disk.
/// 2. Interpolate `${ENV_VAR}` / `${ENV_VAR:-default}` patterns.
/// 3. Parse into a `serde_yaml::Value` tree.
/// 4. Resolve `$include` directives (recursively loading referenced files).
/// 5. Deserialize into `AppConfig`.
/// 6. Run semantic validation.
///
/// # Errors
///
/// Returns `AppError::Config` if the file cannot be read, parsed, or contains
/// invalid YAML. Returns `AppError::Validation` if semantic validation fails.
pub fn load_config(path: &Path) -> Result<AppConfig, AppError> {
    let canonical = path.canonicalize().map_err(|e| {
        AppError::Config(format!(
            "Failed to resolve config path {}: {e}",
            path.display()
        ))
    })?;

    let mut visited = HashSet::new();
    let value = load_yaml_with_includes(&canonical, &mut visited)?;

    let config: AppConfig = serde_yaml::from_value(value)
        .map_err(|e| AppError::Config(format!("Failed to parse YAML config: {e}")))?;

    validate_config(&config)?;

    Ok(config)
}

/// Load a single YAML file, interpolate env vars, parse to Value, and resolve includes.
fn load_yaml_with_includes(path: &Path, visited: &mut HashSet<PathBuf>) -> Result<Value, AppError> {
    // Circular include detection.
    if !visited.insert(path.to_path_buf()) {
        return Err(AppError::Config(format!(
            "Circular $include detected: {} was already included",
            path.display()
        )));
    }

    let raw = std::fs::read_to_string(path).map_err(|e| {
        AppError::Config(format!(
            "Failed to read config file {}: {e}",
            path.display()
        ))
    })?;

    let interpolated = interpolate_env_vars(&raw)?;

    let value: Value = serde_yaml::from_str(&interpolated).map_err(|e| {
        AppError::Config(format!("Failed to parse YAML in {}: {e}", path.display()))
    })?;

    let base_dir = path.parent().unwrap_or(Path::new("."));
    resolve_includes(value, base_dir, visited)
}

/// Walk a `serde_yaml::Value` tree and resolve all `$include` directives.
fn resolve_includes(
    value: Value,
    base_dir: &Path,
    visited: &mut HashSet<PathBuf>,
) -> Result<Value, AppError> {
    match value {
        Value::Mapping(map) => resolve_mapping_includes(map, base_dir, visited),
        Value::Sequence(seq) => resolve_sequence_includes(seq, base_dir, visited),
        // Scalars, nulls, bools, numbers - no includes to resolve.
        other => Ok(other),
    }
}

/// Resolve `$include` in a YAML mapping.
///
/// If the mapping contains a `$include` key:
/// - If it's the *only* key, the include target replaces this mapping entirely
///   (for map-level includes like `tables: { $include: "tables/*.yaml" }`).
/// - The included files must produce mappings, which are merged into the parent.
///
/// All other keys are recursed into normally.
fn resolve_mapping_includes(
    map: serde_yaml::Mapping,
    base_dir: &Path,
    visited: &mut HashSet<PathBuf>,
) -> Result<Value, AppError> {
    let include_key = Value::String("$include".to_string());

    if let Some(include_val) = map.get(&include_key) {
        let pattern = include_val
            .as_str()
            .ok_or_else(|| AppError::Config("$include value must be a string".to_string()))?;

        // A mapping-level $include should be the only key.
        if map.len() > 1 {
            return Err(AppError::Config(
                "$include in a mapping must be the only key (cannot mix with other keys)"
                    .to_string(),
            ));
        }

        let files = resolve_glob(pattern, base_dir)?;
        if files.is_empty() {
            return Err(AppError::Config(format!(
                "$include pattern '{}' matched no files (resolved from {})",
                pattern,
                base_dir.display()
            )));
        }

        // If single file, return its resolved value directly.
        if files.len() == 1 {
            return load_yaml_with_includes(&files[0], visited);
        }

        // Multiple files: merge all into a single mapping.
        let mut merged = serde_yaml::Mapping::new();
        for file in &files {
            let val = load_yaml_with_includes(file, visited)?;
            match val {
                Value::Mapping(m) => {
                    for (k, v) in m {
                        merged.insert(k, v);
                    }
                }
                _ => {
                    return Err(AppError::Config(format!(
                        "$include in a mapping context requires YAML files that produce mappings, \
                         but {} produced a non-mapping value",
                        file.display()
                    )));
                }
            }
        }
        Ok(Value::Mapping(merged))
    } else {
        // No $include - recurse into each value.
        let mut result = serde_yaml::Mapping::new();
        for (k, v) in map {
            result.insert(k, resolve_includes(v, base_dir, visited)?);
        }
        Ok(Value::Mapping(result))
    }
}

/// Resolve `$include` directives within a YAML sequence.
///
/// Sequence items that are `{ $include: "pattern" }` are expanded:
/// - A single-file pattern inserts that file's value at that position.
/// - A glob pattern inserts all matching files' values at that position.
///
/// Other items are recursed into normally.
fn resolve_sequence_includes(
    seq: serde_yaml::Sequence,
    base_dir: &Path,
    visited: &mut HashSet<PathBuf>,
) -> Result<Value, AppError> {
    let include_key = Value::String("$include".to_string());
    let mut result = Vec::new();

    for item in seq {
        if let Value::Mapping(ref map) = item
            && let Some(include_val) = map.get(&include_key)
        {
            let pattern = include_val
                .as_str()
                .ok_or_else(|| AppError::Config("$include value must be a string".to_string()))?;

            if map.len() > 1 {
                return Err(AppError::Config(
                    "$include in a sequence item must be the only key".to_string(),
                ));
            }

            let files = resolve_glob(pattern, base_dir)?;
            if files.is_empty() {
                return Err(AppError::Config(format!(
                    "$include pattern '{}' matched no files (resolved from {})",
                    pattern,
                    base_dir.display()
                )));
            }

            for file in &files {
                let val = load_yaml_with_includes(file, visited)?;
                result.push(val);
            }
            continue;
        }

        // Not an include - recurse.
        result.push(resolve_includes(item, base_dir, visited)?);
    }

    Ok(Value::Sequence(result))
}

/// Resolve a file path or glob pattern relative to a base directory.
///
/// Returns a sorted list of canonical paths. Supports glob patterns like
/// `*.yaml`, `endpoints/*.yaml`, etc.
fn resolve_glob(pattern: &str, base_dir: &Path) -> Result<Vec<PathBuf>, AppError> {
    let full_pattern = base_dir.join(pattern);
    let pattern_str = full_pattern.to_str().ok_or_else(|| {
        AppError::Config(format!(
            "Include path is not valid UTF-8: {}",
            full_pattern.display()
        ))
    })?;

    let mut paths: Vec<PathBuf> = glob::glob(pattern_str)
        .map_err(|e| AppError::Config(format!("Invalid $include glob pattern '{pattern}': {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Config(format!("Error reading $include glob '{pattern}': {e}")))?;

    // Sort for deterministic ordering.
    paths.sort();

    // Canonicalize all paths for circular detection.
    paths
        .into_iter()
        .map(|p| {
            p.canonicalize().map_err(|e| {
                AppError::Config(format!(
                    "Failed to resolve include path {}: {e}",
                    p.display()
                ))
            })
        })
        .collect()
}

/// Replace all `${VAR}` and `${VAR:-default}` occurrences with their env values.
///
/// - `${VAR}` -> value of VAR, or error if unset.
/// - `${VAR:-fallback}` -> value of VAR if set, otherwise `fallback`.
fn interpolate_env_vars(input: &str) -> Result<String, AppError> {
    let re = &*ENV_VAR_RE;

    let mut errors: Vec<String> = Vec::new();
    let result = re.replace_all(input, |caps: &regex::Captures| {
        let var_name = &caps[1];
        match std::env::var(var_name) {
            Ok(val) => val,
            Err(_) => {
                // Check for default value
                if let Some(default_match) = caps.get(2) {
                    default_match.as_str().to_string()
                } else {
                    errors.push(format!(
                        "Environment variable '{var_name}' is not set and has no default"
                    ));
                    format!("${{UNSET_{var_name}}}")
                }
            }
        }
    });

    if !errors.is_empty() {
        return Err(AppError::Config(format!(
            "Environment variable interpolation failed:\n  - {}",
            errors.join("\n  - ")
        )));
    }

    Ok(result.into_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Serializes all env-var-mutating tests so they never run in parallel.
    /// Environment variables are process-global, so concurrent tests can
    /// stomp on each other's state.
    fn env_lock() -> &'static Mutex<()> {
        use std::sync::OnceLock;
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn test_interpolate_with_set_var() {
        let _lock = env_lock().lock().unwrap();
        // SAFETY: test-only code, serialized by env_lock mutex
        unsafe { std::env::set_var("TEST_INTERP_VAR", "hello") };
        let result = interpolate_env_vars("value: ${TEST_INTERP_VAR}").unwrap();
        assert_eq!(result, "value: hello");
        // SAFETY: test-only code, serialized by env_lock mutex
        unsafe { std::env::remove_var("TEST_INTERP_VAR") };
    }

    #[test]
    fn test_interpolate_with_default() {
        let _lock = env_lock().lock().unwrap();
        // SAFETY: test-only code, serialized by env_lock mutex
        unsafe { std::env::remove_var("TEST_MISSING_VAR") };
        let result = interpolate_env_vars("value: ${TEST_MISSING_VAR:-fallback}").unwrap();
        assert_eq!(result, "value: fallback");
    }

    #[test]
    fn test_interpolate_missing_no_default() {
        let _lock = env_lock().lock().unwrap();
        // SAFETY: test-only code, serialized by env_lock mutex
        unsafe { std::env::remove_var("TEST_REQUIRED_VAR_MISSING") };
        let err = interpolate_env_vars("value: ${TEST_REQUIRED_VAR_MISSING}").unwrap_err();
        assert!(
            err.to_string().contains("TEST_REQUIRED_VAR_MISSING"),
            "Error should mention the missing variable name, got: {err}"
        );
    }

    #[test]
    fn test_interpolate_multiple() {
        let _lock = env_lock().lock().unwrap();
        // SAFETY: test-only code, serialized by env_lock mutex
        unsafe {
            std::env::set_var("TEST_A", "aaa");
            std::env::set_var("TEST_B", "bbb");
        }
        let result = interpolate_env_vars("a: ${TEST_A}, b: ${TEST_B}").unwrap();
        assert_eq!(result, "a: aaa, b: bbb");
        // SAFETY: test-only code, serialized by env_lock mutex
        unsafe {
            std::env::remove_var("TEST_A");
            std::env::remove_var("TEST_B");
        }
    }

    #[test]
    fn test_interpolate_no_vars() {
        let input = "just: plain text";
        let result = interpolate_env_vars(input).unwrap();
        assert_eq!(result, input);
    }
}

/// Sub-trait for equality operators (eq / ne).
///
/// Default implementations use `=` for equality and `<>` for inequality,
/// which works for PostgreSQL. MySQL and SQLite override `ne_op` to use `!=`.
pub(crate) trait EqOps {
    fn eq_op(&self) -> &'static str;
    fn ne_op(&self) -> &'static str;
}

/// Sub-trait for comparison operators (gt, gte, lt, lte).
///
/// All three drivers use the same comparison operators.
pub(crate) trait CmpOps {
    fn gt_op(&self) -> &'static str;
    fn gte_op(&self) -> &'static str;
    fn lt_op(&self) -> &'static str;
    fn lte_op(&self) -> &'static str;
}

/// Sub-trait for LIKE operators.
///
/// All drivers use `LIKE` for both `like_op` and `ilike_op` (PostgreSQL adds
/// its own ILIKE support at the query level). `like_pattern_start` and
/// `like_pattern_end` share the same implementation across all drivers.
pub(crate) trait LikeOps {
    fn like_op(&self) -> &'static str;
    fn ilike_op(&self) -> &'static str;
    fn like_pattern(&self, param: &str) -> String;
    fn like_pattern_start(&self, param: &str) -> String;
    fn like_pattern_end(&self, param: &str) -> String;
}

/// Core trait abstracting driver-specific SQL generation patterns for filtering.
///
/// Composed of `EqOps`, `CmpOps`, and `LikeOps` sub-traits for operator
/// resolution, plus driver-specific methods for JSON extraction and
/// JSONB operator detection.
pub(crate) trait FilterBehavior: EqOps + CmpOps + LikeOps {
    /// Generate JSON extraction for nested JSONB/JSON paths.
    fn json_extract_path(&self, column: &str, path: &str) -> String;

    /// Check if driver uses JSONB-specific operators (PostgreSQL).
    fn uses_jsonb_ops(&self) -> bool;
}

/// PostgreSQL-specific filter behavior.
pub(crate) struct PostgresFilter;

impl FilterBehavior for PostgresFilter {
    fn json_extract_path(&self, column: &str, path: &str) -> String {
        let array_syntax: String = path.split('.').collect::<Vec<&str>>().join(",");
        format!("({column} #>> '{{{array_syntax}}}')")
    }

    fn uses_jsonb_ops(&self) -> bool {
        true
    }
}

impl EqOps for PostgresFilter {
    fn eq_op(&self) -> &'static str {
        "="
    }
    fn ne_op(&self) -> &'static str {
        "<>"
    }
}

impl CmpOps for PostgresFilter {
    fn gt_op(&self) -> &'static str {
        ">"
    }
    fn gte_op(&self) -> &'static str {
        ">="
    }
    fn lt_op(&self) -> &'static str {
        "<"
    }
    fn lte_op(&self) -> &'static str {
        "<="
    }
}

impl LikeOps for PostgresFilter {
    fn like_op(&self) -> &'static str {
        "LIKE"
    }
    fn ilike_op(&self) -> &'static str {
        "ILIKE"
    }
    fn like_pattern(&self, param: &str) -> String {
        param.to_string()
    }
    fn like_pattern_start(&self, param: &str) -> String {
        format!("CONCAT({param}, '%')")
    }
    fn like_pattern_end(&self, param: &str) -> String {
        format!("CONCAT('%', {param})")
    }
}

/// MySQL-specific filter behavior.
pub(crate) struct MysqlFilter;

impl FilterBehavior for MysqlFilter {
    fn json_extract_path(&self, column: &str, path: &str) -> String {
        // JSON_EXTRACT returns JSON-encoded values (e.g., quoted strings).
        // JSON_UNQUOTE strips the quotes so comparisons work correctly.
        format!("JSON_UNQUOTE(JSON_EXTRACT({column}, '$.{path}'))")
    }

    fn uses_jsonb_ops(&self) -> bool {
        false
    }
}

impl EqOps for MysqlFilter {
    fn eq_op(&self) -> &'static str {
        "="
    }
    fn ne_op(&self) -> &'static str {
        "!="
    }
}

impl CmpOps for MysqlFilter {
    fn gt_op(&self) -> &'static str {
        ">"
    }
    fn gte_op(&self) -> &'static str {
        ">="
    }
    fn lt_op(&self) -> &'static str {
        "<"
    }
    fn lte_op(&self) -> &'static str {
        "<="
    }
}

impl LikeOps for MysqlFilter {
    fn like_op(&self) -> &'static str {
        "LIKE"
    }
    fn ilike_op(&self) -> &'static str {
        "LIKE"
    }
    fn like_pattern(&self, param: &str) -> String {
        param.to_string()
    }
    fn like_pattern_start(&self, param: &str) -> String {
        format!("CONCAT({param}, '%')")
    }
    fn like_pattern_end(&self, param: &str) -> String {
        format!("CONCAT('%', {param})")
    }
}

/// SQLite-specific filter behavior.
pub(crate) struct SqliteFilter;

impl FilterBehavior for SqliteFilter {
    fn json_extract_path(&self, column: &str, path: &str) -> String {
        format!("json_extract({column}, '$.{path}')")
    }

    fn uses_jsonb_ops(&self) -> bool {
        false
    }
}

impl EqOps for SqliteFilter {
    fn eq_op(&self) -> &'static str {
        "="
    }
    fn ne_op(&self) -> &'static str {
        "!="
    }
}

impl CmpOps for SqliteFilter {
    fn gt_op(&self) -> &'static str {
        ">"
    }
    fn gte_op(&self) -> &'static str {
        ">="
    }
    fn lt_op(&self) -> &'static str {
        "<"
    }
    fn lte_op(&self) -> &'static str {
        "<="
    }
}

impl LikeOps for SqliteFilter {
    fn like_op(&self) -> &'static str {
        "LIKE"
    }
    fn ilike_op(&self) -> &'static str {
        "LIKE"
    }
    fn like_pattern(&self, param: &str) -> String {
        param.to_string()
    }
    fn like_pattern_start(&self, param: &str) -> String {
        format!("CONCAT({param}, '%')")
    }
    fn like_pattern_end(&self, param: &str) -> String {
        format!("CONCAT('%', {param})")
    }
}

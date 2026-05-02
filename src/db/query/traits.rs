/// Trait abstracting driver-specific SQL generation patterns for filtering.
pub trait FilterBehavior {
    /// Generate JSON extraction for nested JSONB/JSON paths.
    fn json_extract_path(&self, table: &str, path: &str) -> String;

    /// Get the comparison operator for equality.
    fn eq_op(&self) -> &'static str;

    /// Get the comparison operator for inequality.
    fn ne_op(&self) -> &'static str;

    /// Get the comparison operator for greater than.
    fn gt_op(&self) -> &'static str;

    /// Get the comparison operator for greater than or equal.
    fn gte_op(&self) -> &'static str;

    /// Get the comparison operator for less than.
    fn lt_op(&self) -> &'static str;

    /// Get the comparison operator for less than or equal.
    fn lte_op(&self) -> &'static str;

    /// Get the LIKE operator.
    fn like_op(&self) -> &'static str;

    /// Get the ILIKE operator (case-insensitive LIKE).
    fn ilike_op(&self) -> &'static str;

    /// Check if driver uses JSONB-specific operators (PostgreSQL).
    fn uses_jsonb_ops(&self) -> bool;
}

/// PostgreSQL-specific filter behavior.
pub struct PostgresFilter;

impl FilterBehavior for PostgresFilter {
    fn json_extract_path(&self, column: &str, path: &str) -> String {
        let array_syntax: String = path.split('.').collect::<Vec<&str>>().join(",");
        format!("({column} #>> '{{{array_syntax}}}')")
    }

    fn eq_op(&self) -> &'static str {
        "="
    }
    fn ne_op(&self) -> &'static str {
        "<>"
    }
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
    fn like_op(&self) -> &'static str {
        "LIKE"
    }
    fn ilike_op(&self) -> &'static str {
        "ILIKE"
    }
    fn uses_jsonb_ops(&self) -> bool {
        true
    }
}

/// MySQL-specific filter behavior.
pub struct MysqlFilter;

impl FilterBehavior for MysqlFilter {
    fn json_extract_path(&self, table: &str, path: &str) -> String {
        format!("JSON_EXTRACT({}, '$.{}')", table, path)
    }

    fn eq_op(&self) -> &'static str {
        "="
    }
    fn ne_op(&self) -> &'static str {
        "!="
    }
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
    fn like_op(&self) -> &'static str {
        "LIKE"
    }
    fn ilike_op(&self) -> &'static str {
        "LIKE COLLATE utf8mb4_general_ci"
    }
    fn uses_jsonb_ops(&self) -> bool {
        false
    }
}

/// SQLite-specific filter behavior.
pub struct SqliteFilter;

impl FilterBehavior for SqliteFilter {
    fn json_extract_path(&self, table: &str, path: &str) -> String {
        format!("json_extract({}, '$.{}')", table, path)
    }

    fn eq_op(&self) -> &'static str {
        "="
    }
    fn ne_op(&self) -> &'static str {
        "!="
    }
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
    fn like_op(&self) -> &'static str {
        "LIKE"
    }
    fn ilike_op(&self) -> &'static str {
        "LIKE"
    }
    fn uses_jsonb_ops(&self) -> bool {
        false
    }
}

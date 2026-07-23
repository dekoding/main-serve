/// Sub-trait for equality operators (eq / ne).
///
/// Default implementations use `=` for equality and `<>` for inequality,
/// which works for PostgreSQL. MySQL and SQLite override `ne_op` to use `!=`.
pub(crate) trait EqOps {
    /// Returns the SQL equality operator string (e.g. `=`).
    fn eq_op(&self) -> &'static str;
    /// Returns the SQL inequality operator string (e.g. `<>` or `!=`).
    fn ne_op(&self) -> &'static str;
}

/// Sub-trait for comparison operators (gt, gte, lt, lte).
///
/// All three drivers use the same comparison operators.
pub(crate) trait CmpOps {
    /// Returns the SQL greater-than operator string.
    fn gt_op(&self) -> &'static str;
    /// Returns the SQL greater-than-or-equal operator string.
    fn gte_op(&self) -> &'static str;
    /// Returns the SQL less-than operator string.
    fn lt_op(&self) -> &'static str;
    /// Returns the SQL less-than-or-equal operator string.
    fn lte_op(&self) -> &'static str;
}

/// Sub-trait for LIKE operators.
///
/// All drivers use `LIKE` for both `like_op` and `ilike_op` (PostgreSQL adds
/// its own ILIKE support at the query level). `like_pattern_start` and
/// `like_pattern_end` share the same implementation across all drivers.
pub(crate) trait LikeOps {
    /// Returns the SQL LIKE operator string.
    fn like_op(&self) -> &'static str;
    /// Returns the SQL ILIKE (case-insensitive) operator string.
    fn ilike_op(&self) -> &'static str;
    /// Returns the LIKE pattern with wildcards applied to both ends of the parameter.
    fn like_pattern(&self, param: &str) -> String;
    /// Returns the LIKE pattern with a wildcard appended to the end of the parameter.
    fn like_pattern_start(&self, param: &str) -> String;
    /// Returns the LIKE pattern with a wildcard prepended to the start of the parameter.
    fn like_pattern_end(&self, param: &str) -> String;
}

/// Core trait abstracting driver-specific SQL generation patterns for filtering.
///
/// Composed of `EqOps`, `CmpOps`, and `LikeOps` sub-traits for operator
/// resolution, plus driver-specific methods for JSON extraction and
/// JSONB operator detection.
pub(crate) trait FilterBehavior: EqOps + CmpOps + LikeOps + std::fmt::Debug {
    /// Generate JSON extraction for nested JSONB/JSON paths.
    fn json_extract_path(&self, column: &str, path: &str) -> String;

    /// Check if driver uses JSONB-specific operators (PostgreSQL).
    fn uses_jsonb_ops(&self) -> bool;
}

/// PostgreSQL-specific filter behavior.
#[derive(Debug)]
pub(crate) struct PostgresFilter;

impl FilterBehavior for PostgresFilter {
    /// Generates the PostgreSQL `#>>` operator syntax for extracting values from JSONB paths.
    fn json_extract_path(&self, column: &str, path: &str) -> String {
        let array_syntax: String = path.split('.').collect::<Vec<&str>>().join(",");
        format!("({column} #>> '{{{array_syntax}}}')")
    }

    /// Returns true, indicating that PostgreSQL uses JSONB-specific operators.
    fn uses_jsonb_ops(&self) -> bool {
        true
    }
}

impl EqOps for PostgresFilter {
    /// Returns the SQL equality operator string `=` for PostgreSQL.
    fn eq_op(&self) -> &'static str {
        "="
    }
    /// Returns the SQL inequality operator string `<>` for PostgreSQL.
    fn ne_op(&self) -> &'static str {
        "<>"
    }
}

impl CmpOps for PostgresFilter {
    /// Returns the SQL greater-than operator string `>` for PostgreSQL.
    fn gt_op(&self) -> &'static str {
        ">"
    }
    /// Returns the SQL greater-than-or-equal operator string `>=` for PostgreSQL.
    fn gte_op(&self) -> &'static str {
        ">="
    }
    /// Returns the SQL less-than operator string `<` for PostgreSQL.
    fn lt_op(&self) -> &'static str {
        "<"
    }
    /// Returns the SQL less-than-or-equal operator string `<=` for PostgreSQL.
    fn lte_op(&self) -> &'static str {
        "<="
    }
}

impl LikeOps for PostgresFilter {
    /// Returns the SQL `LIKE` operator string for PostgreSQL.
    fn like_op(&self) -> &'static str {
        "LIKE"
    }
    /// Returns the SQL `ILIKE` operator string for case-insensitive PostgreSQL matches.
    fn ilike_op(&self) -> &'static str {
        "ILIKE"
    }
    /// Returns the LIKE pattern parameter unchanged for PostgreSQL.
    fn like_pattern(&self, param: &str) -> String {
        param.to_string()
    }
    /// Returns a PostgreSQL CONCAT expression prepending a wildcard to the pattern.
    fn like_pattern_start(&self, param: &str) -> String {
        format!("CONCAT({param}, '%')")
    }
    /// Returns a PostgreSQL CONCAT expression appending a wildcard to the pattern.
    fn like_pattern_end(&self, param: &str) -> String {
        format!("CONCAT('%', {param})")
    }
}
#[derive(Debug)]
pub(crate) struct MysqlFilter;

impl FilterBehavior for MysqlFilter {
    /// Generates a MySQL JSON_UNQUOTE(JSON_EXTRACT(...)) expression for JSON path extraction.
    fn json_extract_path(&self, column: &str, path: &str) -> String {
        // JSON_EXTRACT returns JSON-encoded values (e.g., quoted strings).
        // JSON_UNQUOTE strips the quotes so comparisons work correctly.
        format!("JSON_UNQUOTE(JSON_EXTRACT({column}, '$.{path}'))")
    }

    /// Returns false, indicating that MySQL does not use JSONB-specific operators.
    fn uses_jsonb_ops(&self) -> bool {
        false
    }
}

impl EqOps for MysqlFilter {
    /// Returns the SQL equality operator string `=` for MySQL.
    fn eq_op(&self) -> &'static str {
        "="
    }
    /// Returns the SQL inequality operator string `!=` for MySQL.
    fn ne_op(&self) -> &'static str {
        "!="
    }
}

impl CmpOps for MysqlFilter {
    /// Returns the SQL greater-than operator string `>` for MySQL.
    fn gt_op(&self) -> &'static str {
        ">"
    }
    /// Returns the SQL greater-than-or-equal operator string `>=` for MySQL.
    fn gte_op(&self) -> &'static str {
        ">="
    }
    /// Returns the SQL less-than operator string `<` for MySQL.
    fn lt_op(&self) -> &'static str {
        "<"
    }
    /// Returns the SQL less-than-or-equal operator string `<=` for MySQL.
    fn lte_op(&self) -> &'static str {
        "<="
    }
}

impl LikeOps for MysqlFilter {
    /// Returns the SQL `LIKE` operator string for MySQL.
    fn like_op(&self) -> &'static str {
        "LIKE"
    }
    /// Returns the SQL `LIKE` operator string for MySQL, since MySQL has no ILIKE.
    fn ilike_op(&self) -> &'static str {
        "LIKE"
    }
    /// Returns the LIKE pattern parameter unchanged for MySQL.
    fn like_pattern(&self, param: &str) -> String {
        param.to_string()
    }
    /// Returns a MySQL CONCAT expression prepending a wildcard to the pattern.
    fn like_pattern_start(&self, param: &str) -> String {
        format!("CONCAT({param}, '%')")
    }
    /// Returns a MySQL CONCAT expression appending a wildcard to the pattern.
    fn like_pattern_end(&self, param: &str) -> String {
        format!("CONCAT('%', {param})")
    }
}

/// SQLite-specific filter behavior.
#[derive(Debug)]
pub(crate) struct SqliteFilter;

impl FilterBehavior for SqliteFilter {
    /// Generates a SQLite `json_extract()` function call for JSON path extraction.
    fn json_extract_path(&self, column: &str, path: &str) -> String {
        format!("json_extract({column}, '$.{path}')")
    }

    /// Returns false, indicating that SQLite does not use JSONB-specific operators.
    fn uses_jsonb_ops(&self) -> bool {
        false
    }
}

impl EqOps for SqliteFilter {
    /// Returns the SQL equality operator string `=` for SQLite.
    fn eq_op(&self) -> &'static str {
        "="
    }
    /// Returns the SQL inequality operator string `!=` for SQLite.
    fn ne_op(&self) -> &'static str {
        "!="
    }
}

impl CmpOps for SqliteFilter {
    /// Returns the SQL greater-than operator string `>` for SQLite.
    fn gt_op(&self) -> &'static str {
        ">"
    }
    /// Returns the SQL greater-than-or-equal operator string `>=` for SQLite.
    fn gte_op(&self) -> &'static str {
        ">="
    }
    /// Returns the SQL less-than operator string `<` for SQLite.
    fn lt_op(&self) -> &'static str {
        "<"
    }
    /// Returns the SQL less-than-or-equal operator string `<=` for SQLite.
    fn lte_op(&self) -> &'static str {
        "<="
    }
}

impl LikeOps for SqliteFilter {
    /// Returns the SQL `LIKE` operator string for SQLite.
    fn like_op(&self) -> &'static str {
        "LIKE"
    }
    /// Returns the SQL `LIKE` operator string for SQLite, since SQLite has no ILIKE.
    fn ilike_op(&self) -> &'static str {
        "LIKE"
    }
    /// Returns the LIKE pattern parameter unchanged for SQLite.
    fn like_pattern(&self, param: &str) -> String {
        param.to_string()
    }
    /// Returns a SQLite CONCAT expression prepending a wildcard to the pattern.
    fn like_pattern_start(&self, param: &str) -> String {
        format!("CONCAT({param}, '%')")
    }
    /// Returns a SQLite CONCAT expression appending a wildcard to the pattern.
    fn like_pattern_end(&self, param: &str) -> String {
        format!("CONCAT('%', {param})")
    }
}

use crate::config::types::DatabaseDriver;
use std::sync::LazyLock;

/// Compiled regex to detect common SQL keywords and injection patterns.
static SQL_KEYWORDS_RE: LazyLock<Option<regex::Regex>> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)\b(or|and|union|select|insert|update|delete|drop|alter|create|truncate|exec)\b",
    )
    .ok()
});

/// Validate that a string is a safe SQL expression (for JSONB computed fields).
///
/// This function supports:
/// 1. Formal `JSONPath` syntax (via the `jsonb` crate).
/// 2. Standard SQL/PostgreSQL JSONB operators (e.g., `->`, `->>`, `#>`, `#>>`).
/// 3. Bracket notation for sorting (e.g., `metadata[role]`).
#[must_use]
pub(crate) fn is_valid_expression(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    if has_forbidden_patterns(s) {
        return false;
    }
    if is_valid_json_path(s) {
        return true;
    }
    // If the string uses bracket notation, it must be well-formed.
    if super::sorting::is_bracket_notation(s) && !is_valid_bracket_notation(s) {
        return false;
    }
    if !has_balanced_parens(s) {
        return false;
    }
    if !has_balanced_quotes(s) {
        return false;
    }
    if has_dangerous_sql_keywords(s) {
        return false;
    }
    if has_dangerous_function_calls(s) {
        return false;
    }
    s.chars()
        .all(|c| c.is_alphanumeric() || "_.,()=<>+-*/'#[]".contains(c))
}

/// Validate that a string is a safe SQL identifier (prevents injection).
/// Allows alphanumeric, underscores, dots (for table.column), and hyphens.
#[must_use]
pub(crate) fn is_valid_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// Quote an object name (table or index) for the current driver.
#[inline]
/// quote_identifier
pub fn quote_identifier(name: &str, driver: DatabaseDriver) -> String {
    match driver {
        DatabaseDriver::Mysql => format!("`{name}`"),
        DatabaseDriver::Sqlite | DatabaseDriver::Postgres => format!("\"{name}\""),
    }
}

/// Check for forbidden SQL injection patterns (comments, semicolons, block comments).
fn has_forbidden_patterns(s: &str) -> bool {
    let forbidden = [";", "--", "/*", "*/"];
    forbidden.iter().any(|&p| s.contains(p))
}

/// Check if string is valid JSONPath syntax (parsed by the jsonb crate).
fn is_valid_json_path(s: &str) -> bool {
    jsonb::jsonpath::parse_json_path(s.as_bytes()).is_ok()
}

/// Check if string uses bracket notation and validate it is well-formed.
fn is_valid_bracket_notation(s: &str) -> bool {
    if !super::sorting::is_bracket_notation(s) {
        return false;
    }
    let mut bracket_depth = 0;
    let mut in_bracket = false;
    for c in s.chars() {
        if c == '[' {
            bracket_depth += 1;
            in_bracket = true;
        } else if c == ']' {
            bracket_depth -= 1;
            in_bracket = false;
        } else if in_bracket && !c.is_alphanumeric() && c != '_' {
            return false;
        }
    }
    bracket_depth == 0
}

/// Check for balanced parentheses.
fn has_balanced_parens(s: &str) -> bool {
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// Check for balanced single quotes.
fn has_balanced_quotes(s: &str) -> bool {
    s.chars().filter(|&c| c == '\'').count() % 2 == 0
}

/// Check for dangerous SQL keywords using regex.
fn has_dangerous_sql_keywords(s: &str) -> bool {
    SQL_KEYWORDS_RE.as_ref().is_some_and(|re| re.is_match(s))
}

/// Compiled regex to detect dangerous SQL Server extended procedure calls (xp_*) and
/// sleep-based time injection (`pg_sleep`, SLEEP, etc.).
static DANGEROUS_FN_RE: LazyLock<Option<regex::Regex>> =
    LazyLock::new(|| regex::Regex::new(r"(?i)(\bxp_\w+|\bsleep\s*\(|_sleep\b)").ok());

/// Check for dangerous function calls (xp_, pg_sleep, etc.) using regex.
fn has_dangerous_function_calls(s: &str) -> bool {
    DANGEROUS_FN_RE.as_ref().is_some_and(|re| re.is_match(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- is_valid_identifier --

    #[test]
    fn test_is_valid_identifier() {
        assert!(!is_valid_identifier("DROP TABLE;--"));
        assert!(!is_valid_identifier("field; DELETE"));
        assert!(is_valid_identifier("user_name"));
        assert!(is_valid_identifier("table.column"));
        assert!(is_valid_identifier("table-column"));
        assert!(!is_valid_identifier(""));
    }

    #[test]
    fn test_is_valid_identifier_special_chars() {
        assert!(!is_valid_identifier("field name")); // space
        assert!(!is_valid_identifier("field'or'1=1")); // quote
        assert!(is_valid_identifier("field--comment"));
    }

    // -- is_valid_expression --

    #[test]
    fn test_is_valid_expression_safe_simple() {
        assert!(is_valid_expression("title"));
        assert!(is_valid_expression("metadata.role"));
        assert!(is_valid_expression("metadata[role]"));
    }

    #[test]
    fn test_is_valid_expression_safe_jsonpath() {
        assert!(is_valid_expression("$.role"));
        assert!(is_valid_expression("$.metadata.role"));
    }

    #[test]
    fn test_is_valid_expression_safe_operators() {
        assert!(is_valid_expression("metadata->>'role'"));
        assert!(!is_valid_expression("metadata#>'{role}'"));
    }

    #[test]
    fn test_is_valid_expression_semicolon_blocked() {
        assert!(!is_valid_expression("title; DROP TABLE"));
        assert!(!is_valid_expression("metadata[role];evil"));
    }

    #[test]
    fn test_is_valid_expression_comments_blocked() {
        assert!(!is_valid_expression("--comment"));
        assert!(!is_valid_expression("/* comment */"));
        assert!(!is_valid_expression("*/inject"));
    }

    #[test]
    fn test_is_valid_expression_sql_with_semicolon() {
        assert!(!is_valid_expression("title; SELECT * FROM users"));
    }

    #[test]
    fn test_is_valid_expression_unbalanced_brackets() {
        assert!(!is_valid_expression("metadata["));
        assert!(is_valid_expression("metadata]"));
        assert!(!is_valid_expression("metadata[role"));
    }

    #[test]
    fn test_is_valid_expression_bracket_invalid_chars() {
        assert!(!is_valid_expression("metadata[role;drop]"));
        assert!(!is_valid_expression("metadata[role'or'1=1]"));
    }

    #[test]
    fn test_is_valid_expression_unbalanced_parens() {
        assert!(!is_valid_expression("metadata((role"));
        assert!(!is_valid_expression("metadata)))"));
        assert!(!is_valid_expression("metadata[role)"));
    }

    #[test]
    fn test_is_valid_expression_odd_quotes() {
        assert!(!is_valid_expression("metadata'role"));
        assert!(is_valid_expression("'metadata'"));
    }

    #[test]
    fn test_is_valid_expression_empty() {
        assert!(!is_valid_expression(""));
    }

    #[test]
    fn test_is_valid_expression_drop_table() {
        assert!(!is_valid_expression("DROP TABLE posts"));
        assert!(!is_valid_expression("DELETE FROM posts"));
        assert!(!is_valid_expression("TRUNCATE TABLE posts"));
        assert!(!is_valid_expression("ALTER TABLE posts DROP COLUMN id"));
        assert!(!is_valid_expression("DELETE; FROM posts"));
    }

    #[test]
    fn test_is_valid_expression_injection_with_comments() {
        assert!(!is_valid_expression("'; DROP TABLE posts--"));
        assert!(!is_valid_expression("admin'--"));
        assert!(!is_valid_expression("' OR '1'='1"));
    }

    #[test]
    fn test_is_valid_expression_rejects_spaces() {
        assert!(!is_valid_expression("metadata . role"));
    }

    #[test]
    fn test_is_valid_expression_sql_functions() {
        assert!(!is_valid_expression("pg_sleep(10)"));
        assert!(!is_valid_expression("xp_cmdshell('dir')"));
        assert!(!is_valid_expression("SLEEP(5)"));
    }

    // -- quote_identifier --

    #[test]
    fn test_quote_identifier_sqlite() {
        assert_eq!(
            quote_identifier("users", DatabaseDriver::Sqlite),
            r#""users""#
        );
        assert_eq!(
            quote_identifier("table_name", DatabaseDriver::Sqlite),
            r#""table_name""#
        );
    }

    #[test]
    fn test_quote_identifier_postgres() {
        assert_eq!(
            quote_identifier("users", DatabaseDriver::Postgres),
            r#""users""#
        );
        assert_eq!(
            quote_identifier("table.name", DatabaseDriver::Postgres),
            r#""table.name""#
        );
    }

    #[test]
    fn test_quote_identifier_mysql() {
        assert_eq!(quote_identifier("users", DatabaseDriver::Mysql), "`users`");
        assert_eq!(
            quote_identifier("table_name", DatabaseDriver::Mysql),
            "`table_name`"
        );
    }

    // -- has_balanced_parens (private function) --

    #[test]
    fn test_has_balanced_parens_balanced() {
        assert!(has_balanced_parens("foo(bar)"));
        assert!(has_balanced_parens("foo(bar(baz))"));
        assert!(has_balanced_parens(""));
    }

    #[test]
    fn test_has_balanced_parens_unbalanced() {
        assert!(!has_balanced_parens("(foo"));
        assert!(!has_balanced_parens("foo)"));
        assert!(!has_balanced_parens("((foo)"));
    }

    // -- has_balanced_quotes (private function) --

    #[test]
    fn test_has_balanced_quotes_even() {
        assert!(has_balanced_quotes("''"));
        assert!(has_balanced_quotes("'foo'"));
        assert!(has_balanced_quotes("'foo''bar'"));
    }

    #[test]
    fn test_has_balanced_quotes_odd() {
        assert!(!has_balanced_quotes("'foo"));
        assert!(!has_balanced_quotes("foo'"));
        assert!(!has_balanced_quotes("foo'bar'baz'"));
    }

    // -- has_forbidden_patterns (private function) --

    #[test]
    fn test_has_forbidden_patterns_semicolon() {
        assert!(has_forbidden_patterns("foo;bar"));
    }

    #[test]
    fn test_has_forbidden_patterns_comments() {
        assert!(has_forbidden_patterns("--comment"));
        assert!(has_forbidden_patterns("/* comment */"));
    }

    #[test]
    fn test_has_forbidden_patterns_clean() {
        assert!(!has_forbidden_patterns("foo.bar"));
        assert!(!has_forbidden_patterns("metadata[role]"));
    }

    // -- Integration: full validation pipeline --

    #[test]
    fn test_validation_pipeline_safe_identifiers() {
        let identifiers = ["user_name", "table.column", "table-column", "field_1"];
        for id in identifiers {
            assert!(is_valid_identifier(id), "expected '{id}' to be valid");
        }
    }

    #[test]
    fn test_validation_pipeline_unsafe_identifiers() {
        let identifiers = [
            "DROP TABLE;--",
            "field; DELETE",
            "field name",
            "field'or'1=1",
            "",
        ];
        for id in identifiers {
            assert!(!is_valid_identifier(id), "expected '{id}' to be invalid");
        }
    }

    #[test]
    fn test_validation_pipeline_safe_expressions() {
        let expressions = [
            "title",
            "metadata.role",
            "metadata[role]",
            "$.role",
            "metadata->>'role'",
        ];
        for expr in expressions {
            assert!(is_valid_expression(expr), "expected '{expr}' to be valid");
        }
    }

    #[test]
    fn test_validation_pipeline_injection_attempts() {
        let expressions = [
            "title; DROP TABLE",
            "--comment",
            "/* comment */",
            "DROP TABLE posts",
            "DELETE FROM posts",
            "pg_sleep(10)",
            "xp_cmdshell('dir')",
            "' OR '1'='1",
            "metadata[role;drop]",
            "metadata[role'or'1=1]",
            "metadata((role",
            "metadata)))",
            "metadata'role",
            "metadata . role",
        ];
        for expr in expressions {
            assert!(
                !is_valid_expression(expr),
                "expected '{expr}' to be blocked"
            );
        }
    }
}

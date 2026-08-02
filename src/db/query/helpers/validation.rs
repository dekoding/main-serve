//! Query validation utilities for columns and filter fields.
//! Characters allowed in SQL expressions from config (join ON clauses,
//! computed field expressions, where clauses). This rejects semicolons,
//! comments, backticks, command substitution, newlines, and other dangerous
//! SQL metacharacters while still allowing typical expressions like
//! `table.col = other.col` or `COUNT(*)`, and interpolation syntax like
//! `${request.user.id}`.
#[must_use]
pub fn is_safe_sql_fragment(s: &str) -> bool {
    !s.is_empty()
        && !s.contains(';')
        && !s.contains("--")
        && !s.contains("/*")
        && !s.contains('`')
        && !s.contains("$(")
        && !s.contains('\n')
        && !s.contains('\r')
}

use crate::db::query::helpers::is_bracket_notation;
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
pub fn is_valid_expression(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    // Check for forbidden SQL injection patterns (comments, semicolons, block comments).
    let forbidden = [";", "--", "/*", "*/"];
    if forbidden.iter().any(|&p| s.contains(p)) {
        return false;
    }
    // Check if string is valid JSONPath syntax (parsed by the jsonb crate).
    if jsonb::jsonpath::parse_json_path(s.as_bytes()).is_ok() {
        return true;
    }
    // If the string uses bracket notation, it must be well-formed.
    let mut bracket_depth = 0;
    if is_bracket_notation(s) {
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
    }
    if bracket_depth != 0 {
        return false;
    }

    // Check for balanced parentheses.
    let mut paren_depth = 0;
    for c in s.chars() {
        match c {
            '(' => paren_depth += 1,
            ')' => {
                paren_depth -= 1;
                if paren_depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    if paren_depth != 0 {
        return false;
    }
    // Check for balanced single quotes.
    if s.chars().filter(|&c| c == '\'').count() % 2 != 0 {
        return false;
    }
    // Check for dangerous SQL keywords using regex.
    if SQL_KEYWORDS_RE.as_ref().is_some_and(|re| re.is_match(s)) {
        return false;
    }
    // Check for dangerous function calls (xp_, pg_sleep, etc.) using regex.
    if DANGEROUS_FN_RE.as_ref().is_some_and(|re| re.is_match(s)) {
        return false;
    }
    s.chars()
        .all(|c| c.is_alphanumeric() || "_.,()=<>+-*/'#[]".contains(c))
}

/// Validate that a string is a safe SQL identifier (prevents injection).
/// Allows alphanumeric, underscores, dots (for table.column), and hyphens.
#[must_use]
pub fn is_valid_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// Compiled regex to detect dangerous SQL Server extended procedure calls (xp_*) and
/// sleep-based time injection (`pg_sleep`, SLEEP, etc.).
static DANGEROUS_FN_RE: LazyLock<Option<regex::Regex>> =
    LazyLock::new(|| regex::Regex::new(r"(?i)(\bxp_\w+|\bsleep\s*\(|_sleep\b)").ok());

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

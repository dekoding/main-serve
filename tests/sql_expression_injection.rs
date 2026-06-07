//! # SQL Expression Injection Adversarial Tests
//!
//! This test suite attempts to exploit vulnerabilities in the `is_valid_expression`
//! function used to validate sorting fields and computed field expressions.
//!
//! The tests attempt to bypass expression validation through:
//! 1. SQL comment injection
//! 2. SQL keyword injection
//! 3. Function call abuse
//! 4. Operator chaining
//! 5. `JSONPath` parser bypass
//! 6. Context-aware validation gaps
//!
//! If any of these tests pass (i.e., malicious expressions are accepted),
//! they represent security vulnerabilities that must be addressed.

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use crate::support::configs::shared_configs::JSONB_EXPRESSIONS_CONFIG;
use crate::support::db::enabled_backends;

// =============================================================================
// Test Configuration: JSONB-enabled table for expression testing
// =============================================================================

async fn seed_jsonb_posts(app: &axum::Router, posts: &[(&str, &str, &str)]) {
    for (title, author, metadata_json) in posts {
        let metadata_value: serde_json::Value =
            serde_json::from_str(metadata_json).unwrap_or_else(|e| {
                panic!(
                    "Failed to parse metadata JSON for post '{title}': {e}\nJSON input: {metadata_json}"
                );
            });

        let req = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "title": title,
                    "author": author,
                    "metadata": metadata_value
                })
                .to_string(),
            ))
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "Failed to seed post: '{title}'"
        );
    }
}

// =============================================================================
// Test 1: SQL Comment Injection with Parentheses
// =============================================================================

#[tokio::test]
async fn test_sql_comment_injection_in_sort() {
    for backend in enabled_backends() {
        let mut test_db = support::db::TestDatabase::new(backend, "sql_comment_injection");
        let (app, _state) = test_db
            .setup_app(JSONB_EXPRESSIONS_CONFIG, "comment_test.yaml")
            .await;

        seed_jsonb_posts(
            &app,
            &[
                ("Post 1", "Alice", r#"{"role": "admin"}"#),
                ("Post 2", "Bob", r#"{"role": "user"}"#),
                ("Post 3", "Charlie", r#"{"role": "guest"}"#),
            ],
        )
        .await;

        // Test: SQL comment injection attempt
        // The expression "title" should be safe, but we're testing if "UPPER(title) --" passes
        let malicious_sort = "UPPER(title) --";
        let req = Request::builder()
            .uri(format!(
                "/api/posts?sort={}&order=asc",
                urlencoding_encode(malicious_sort)
            ))
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();

        // This should be rejected because it contains SQL comment syntax
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "backend: {backend} - SQL comment injection in sort field should be rejected"
        );
    }
}

// =============================================================================
// Test 2: SQL Keyword Injection
// =============================================================================

#[tokio::test]
async fn test_sql_keyword_injection_in_sort() {
    for backend in enabled_backends() {
        let mut test_db = support::db::TestDatabase::new(backend, "sql_keyword_injection");
        let (app, _state) = test_db
            .setup_app(JSONB_EXPRESSIONS_CONFIG, "keyword_test.yaml")
            .await;

        seed_jsonb_posts(
            &app,
            &[("Post 1", "Alice", r"{}"), ("Post 2", "Bob", r"{}")],
        )
        .await;

        // Test: UNION keyword injection
        let malicious_sort = "title UNION SELECT * FROM posts";
        let req = Request::builder()
            .uri(format!(
                "/api/posts?sort={}&order=asc",
                urlencoding_encode(malicious_sort)
            ))
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();

        // This should be rejected because it contains UNION keyword
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "backend: {backend} - UNION injection in sort field should be rejected"
        );
    }
}

// =============================================================================
// Test 3: Function Call Abuse (pg_sleep time-based injection)
// =============================================================================

#[tokio::test]
async fn test_function_call_abuse_in_sort() {
    for backend in enabled_backends() {
        let mut test_db = support::db::TestDatabase::new(backend, "function_abuse");
        let (app, _state) = test_db
            .setup_app(JSONB_EXPRESSIONS_CONFIG, "function_test.yaml")
            .await;

        seed_jsonb_posts(
            &app,
            &[("Post 1", "Alice", r"{}"), ("Post 2", "Bob", r"{}")],
        )
        .await;

        // Test: pg_sleep function injection (time-based blind SQL injection)
        // This shouldn't be accepted as a valid sort field
        let malicious_sort = "pg_sleep(10)";
        let req = Request::builder()
            .uri(format!(
                "/api/posts?sort={}&order=asc",
                urlencoding_encode(malicious_sort)
            ))
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();

        // This should be rejected - pg_sleep is not a valid sort expression
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "backend: {backend} - pg_sleep function should be rejected in sort field"
        );
    }
}

// =============================================================================
// Test 4: Operator Chaining Injection
// =============================================================================

#[tokio::test]
async fn test_operator_chaining_in_sort() {
    for backend in enabled_backends() {
        let mut test_db = support::db::TestDatabase::new(backend, "operator_chaining");
        let (app, _state) = test_db
            .setup_app(JSONB_EXPRESSIONS_CONFIG, "operator_test.yaml")
            .await;

        seed_jsonb_posts(
            &app,
            &[("Post 1", "Alice", r"{}"), ("Post 2", "Bob", r"{}")],
        )
        .await;

        // Test: OR/AND operator chaining
        let malicious_sort = "title = 'a' AND 1=1";
        let req = Request::builder()
            .uri(format!(
                "/api/posts?sort={}&order=asc",
                urlencoding_encode(malicious_sort)
            ))
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();

        // This should be rejected - it's not a valid sort field
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "backend: {backend} - operator chaining in sort field should be rejected"
        );
    }
}

// =============================================================================
// Test 5: JSONPath Parser Bypass
// =============================================================================

#[tokio::test]
async fn test_jsonpath_parser_bypass() {
    for backend in enabled_backends() {
        let mut test_db = support::db::TestDatabase::new(backend, "jsonpath_bypass");
        let (app, _state) = test_db
            .setup_app(JSONB_EXPRESSIONS_CONFIG, "jsonpath_test.yaml")
            .await;

        seed_jsonb_posts(
            &app,
            &[
                ("Post 1", "Alice", r#"{"role": "admin"}"#),
                ("Post 2", "Bob", r#"{"role": "user"}"#),
            ],
        )
        .await;

        // Test: Complex JSONPath that might bypass validation
        let malicious_jsonpath = "$.metadata.role[*]";
        let req = Request::builder()
            .uri(format!(
                "/api/posts?sort={}&order=asc",
                urlencoding_encode(malicious_jsonpath)
            ))
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();

        // Complex JSONPath patterns should be rejected as invalid sort expressions.
        // If accepted, the returned data must not expose fields outside allowed_fields.
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "backend: {backend} - complex JSONPath bypass attempt should be rejected"
        );
    }
}

// =============================================================================
// Test 6: Whitespace-Obfuscated Injection
// =============================================================================

#[tokio::test]
async fn test_whitespace_obfuscated_injection() {
    for backend in enabled_backends() {
        let mut test_db = support::db::TestDatabase::new(backend, "whitespace_obfuscation");
        let (app, _state) = test_db
            .setup_app(JSONB_EXPRESSIONS_CONFIG, "whitespace_test.yaml")
            .await;

        seed_jsonb_posts(
            &app,
            &[("Post 1", "Alice", r"{}"), ("Post 2", "Bob", r"{}")],
        )
        .await;

        // Test: Multi-line SQL injection attempt
        // This would be encoded as: title%0A%3B%20DROP%20TABLE%20posts
        let malicious_sort = "title\n;\nDROP TABLE posts";
        let req = Request::builder()
            .uri(format!(
                "/api/posts?sort={}&order=asc",
                urlencoding_encode(malicious_sort)
            ))
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();

        // This should be rejected - newline characters are not allowed
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "backend: {backend} - newline-obfuscated injection should be rejected"
        );
    }
}

// =============================================================================
// Test 7: PostgreSQL JSONB Operator Abuse
// =============================================================================

#[tokio::test]
async fn test_postgres_jsonb_operator_abuse() {
    for backend in enabled_backends() {
        let mut test_db = support::db::TestDatabase::new(backend, "jsonb_operator_abuse");
        let (app, _state) = test_db
            .setup_app(JSONB_EXPRESSIONS_CONFIG, "jsonb_operator_test.yaml")
            .await;

        seed_jsonb_posts(
            &app,
            &[
                (
                    "Post 1",
                    "Alice",
                    r#"{"role": "admin", "secret": "sensitive"}"#,
                ),
                (
                    "Post 2",
                    "Bob",
                    r#"{"role": "user", "secret": "also_sensitive"}"#,
                ),
            ],
        )
        .await;

        // Test: Attempt to abuse #>> operator
        let malicious_sort = "metadata#>>'{secret}'";
        let req = Request::builder()
            .uri(format!(
                "/api/posts?sort={}&order=asc",
                urlencoding_encode(malicious_sort)
            ))
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();

        // The #>> operator is not a valid sort expression. It should be rejected.
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "backend: {backend} - JSONB #>> operator abuse should be rejected as invalid sort field"
        );
    }
}

// =============================================================================
// Test 8: Current User/Context Keyword Injection
// =============================================================================

#[tokio::test]
async fn test_context_keyword_injection() {
    for backend in enabled_backends() {
        let mut test_db = support::db::TestDatabase::new(backend, "context_keyword");
        let (app, _state) = test_db
            .setup_app(JSONB_EXPRESSIONS_CONFIG, "context_test.yaml")
            .await;

        seed_jsonb_posts(
            &app,
            &[("Post 1", "Alice", r"{}"), ("Post 2", "Bob", r"{}")],
        )
        .await;

        // Test: PostgreSQL system functions
        let malicious_sort = "current_user";
        let req = Request::builder()
            .uri(format!(
                "/api/posts?sort={}&order=asc",
                urlencoding_encode(malicious_sort)
            ))
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();

        // This should be rejected - not a valid column name
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "backend: {backend} - current_user keyword should be rejected in sort field"
        );
    }
}

// =============================================================================
// Test 9: Quote Imbalance Exploitation
// =============================================================================

#[tokio::test]
async fn test_quote_imbalance_exploitation() {
    for backend in enabled_backends() {
        let mut test_db = support::db::TestDatabase::new(backend, "quote_imbalance");
        let (app, _state) = test_db
            .setup_app(JSONB_EXPRESSIONS_CONFIG, "quote_test.yaml")
            .await;

        seed_jsonb_posts(
            &app,
            &[("Post 1", "Alice", r"{}"), ("Post 2", "Bob", r"{}")],
        )
        .await;

        // Test: Quote manipulation
        let malicious_sort = "' OR '1'='1";
        let req = Request::builder()
            .uri(format!(
                "/api/posts?sort={}&order=asc",
                urlencoding_encode(malicious_sort)
            ))
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();

        // This should be rejected - malformed quote handling
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "backend: {backend} - quote imbalance should be rejected"
        );
    }
}

// =============================================================================
// Test 10: Nested Comment Injection
// =============================================================================

#[tokio::test]
async fn test_nested_comment_injection() {
    for backend in enabled_backends() {
        let mut test_db = support::db::TestDatabase::new(backend, "nested_comment");
        let (app, _state) = test_db
            .setup_app(JSONB_EXPRESSIONS_CONFIG, "nested_comment_test.yaml")
            .await;

        seed_jsonb_posts(
            &app,
            &[("Post 1", "Alice", r"{}"), ("Post 2", "Bob", r"{}")],
        )
        .await;

        // Test: Nested comment attempt
        let malicious_sort = "title/* comment */ desc";
        let req = Request::builder()
            .uri(format!(
                "/api/posts?sort={}&order=asc",
                urlencoding_encode(malicious_sort)
            ))
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();

        // This should be rejected - contains /* */ comment syntax
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "backend: {backend} - nested comment syntax should be rejected"
        );
    }
}

// =============================================================================
// Helper function to URL-encode sort parameters
// =============================================================================

fn urlencoding_encode(s: &str) -> String {
    use std::fmt::Write;
    let mut result = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
            write!(&mut result, "{c}").unwrap();
        } else {
            write!(&mut result, "%{:02X}", c as u32).unwrap();
        }
    }
    result
}

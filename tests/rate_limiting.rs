use crate::support::configs::middleware_configs::RATE_LIMIT_CONFIG;
use axum::{body::Body, extract::Request};
use http::StatusCode;
use tower::ServiceExt;

mod support;

#[tokio::test]
async fn test_rate_limit_blocks_excess_requests() {
    let (app, _f) = support::setup_server(RATE_LIMIT_CONFIG).await;

    // First 3 requests should succeed.
    for i in 0..3 {
        let req = Request::builder()
            .uri("/limited")
            .header("X-Client-Id", "client-1")
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "Request {i} should succeed"
        );
    }

    // 4th request should be rate limited.
    let req = Request::builder()
        .uri("/limited")
        .header("X-Client-Id", "client-1")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn test_rate_limit_different_clients_independent() {
    let (app, _f) = support::setup_server(RATE_LIMIT_CONFIG).await;

    // Exhaust client-a's limit.
    for _ in 0..3 {
        let req = Request::builder()
            .uri("/limited")
            .header("X-Client-Id", "client-a")
            .body(Body::empty())
            .unwrap();
        app.clone().oneshot(req).await.unwrap();
    }

    // client-b should still be allowed.
    let req = Request::builder()
        .uri("/limited")
        .header("X-Client-Id", "client-b")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

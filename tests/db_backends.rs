mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use support::db::{TestDatabase, enabled_backends};
use support::{CRUD_CONFIG, json_body};

#[tokio::test]
async fn test_crud_create_and_list_across_backends() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_create_and_list");
        let app = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        let create_request = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "title": "Hello World",
                    "body": "First post!",
                    "author": "Alice"
                })
                .to_string(),
            ))
            .unwrap();

        let create_response = app.clone().oneshot(create_request).await.unwrap();
        assert_eq!(
            create_response.status(),
            StatusCode::CREATED,
            "backend: {backend}"
        );

        let list_request = Request::builder()
            .uri("/api/posts")
            .body(Body::empty())
            .unwrap();

        let list_response = app.oneshot(list_request).await.unwrap();
        assert_eq!(list_response.status(), StatusCode::OK, "backend: {backend}");

        let json = json_body(list_response).await;
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 1, "backend: {backend}");
        assert_eq!(data[0]["title"], "Hello World", "backend: {backend}");
        assert_eq!(data[0]["author"], "Alice", "backend: {backend}");
    }
}

#[tokio::test]
async fn test_crud_update_and_delete_across_backends() {
    for backend in enabled_backends() {
        let test_db = TestDatabase::new(backend, "crud_update_and_delete");
        let app = test_db.setup_app(CRUD_CONFIG, "crud.yaml").await;

        let create_request = Request::builder()
            .method("POST")
            .uri("/api/posts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "title": "Original",
                    "body": "body",
                    "author": "Bob"
                })
                .to_string(),
            ))
            .unwrap();
        let create_response = app.clone().oneshot(create_request).await.unwrap();
        assert_eq!(
            create_response.status(),
            StatusCode::CREATED,
            "backend: {backend}"
        );

        let update_request = Request::builder()
            .method("PUT")
            .uri("/api/posts/1")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"title": "Updated", "body": "new body"}).to_string(),
            ))
            .unwrap();
        let update_response = app.clone().oneshot(update_request).await.unwrap();
        assert_eq!(
            update_response.status(),
            StatusCode::OK,
            "backend: {backend}"
        );

        let get_request = Request::builder()
            .uri("/api/posts/1")
            .body(Body::empty())
            .unwrap();
        let get_response = app.clone().oneshot(get_request).await.unwrap();
        assert_eq!(get_response.status(), StatusCode::OK, "backend: {backend}");
        let json = json_body(get_response).await;
        assert_eq!(json["title"], "Updated", "backend: {backend}");

        let delete_request = Request::builder()
            .method("DELETE")
            .uri("/api/posts/1")
            .body(Body::empty())
            .unwrap();
        let delete_response = app.clone().oneshot(delete_request).await.unwrap();
        assert_eq!(
            delete_response.status(),
            StatusCode::OK,
            "backend: {backend}"
        );

        let list_request = Request::builder()
            .uri("/api/posts")
            .body(Body::empty())
            .unwrap();
        let list_response = app.oneshot(list_request).await.unwrap();
        let json = json_body(list_response).await;
        let data = json["data"].as_array().unwrap();
        assert!(data.is_empty(), "backend: {backend}");
    }
}

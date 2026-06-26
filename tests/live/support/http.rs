//! HTTP client for live tests.
//!
//! Wraps [`reqwest::Client`] with a base URL and common helper methods
//! for JSON requests, authentication, and multipart uploads.

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Response, StatusCode};

/// HTTP client configured for a specific Main Serve instance.
pub struct LiveClient {
    client: reqwest::Client,
    base_url: String,
    default_headers: HeaderMap,
}

impl LiveClient {
    /// Create a new client targeting `base_url`.
    pub fn new(base_url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            default_headers: HeaderMap::new(),
        }
    }

    /// Set a default header that will be included in every request.
    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.default_headers.insert(
            HeaderName::from_bytes(key.as_bytes()).expect("invalid header name"),
            HeaderValue::from_str(value).expect("invalid header value"),
        );
        self
    }

    /// Set the `Authorization` header (Bearer token).
    pub fn with_bearer_token(mut self, token: &str) -> Self {
        self.default_headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).expect("invalid token"),
        );
        self
    }

    /// Build the full URL for a path.
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

   /// Get a reference to the underlying reqwest client.
   pub fn client(&self) -> &reqwest::Client {
       &self.client
   }

    /// Perform a GET request and return the response.
    pub async fn get(&self, path: &str) -> reqwest::Result<Response> {
        self.client
            .get(self.url(path))
            .headers(self.default_headers.clone())
            .send()
            .await
    }

    /// Perform a GET request and parse the JSON body.
    pub async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> reqwest::Result<T> {
        let resp = self.get(path).await?;
        resp.json::<T>().await
    }

    /// Perform a HEAD request (useful for testing static file metadata).
    pub async fn head(&self, path: &str) -> reqwest::Result<Response> {
        self.client
            .head(self.url(path))
            .headers(self.default_headers.clone())
            .send()
            .await
    }

    /// Perform a POST request with a JSON body.
    pub async fn post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> reqwest::Result<Response> {
        self.client
            .post(self.url(path))
            .headers(self.default_headers.clone())
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await
    }

    /// Perform a PUT request with a JSON body.
    pub async fn put_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> reqwest::Result<Response> {
        self.client
            .put(self.url(path))
            .headers(self.default_headers.clone())
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await
    }

    /// Perform a PATCH request with a JSON body.
    pub async fn patch_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> reqwest::Result<Response> {
        self.client
            .patch(self.url(path))
            .headers(self.default_headers.clone())
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await
    }

    /// Perform a DELETE request.
    pub async fn delete(&self, path: &str) -> reqwest::Result<Response> {
        self.client
            .delete(self.url(path))
            .headers(self.default_headers.clone())
            .send()
            .await
    }

    /// Assert that the response has the expected status code.
    ///
    /// Panics with the status code on mismatch.
    pub async fn assert_status(&self, resp: &Response, expected: StatusCode) {
        let status = resp.status();
        if status != expected {
            panic!(
                "Expected status {}, got {}. Response body: {}",
                expected, status, "<body not available>"
            );
        }
    }

    /// Assert that the response has the expected status code and return it.
    ///
    /// Convenience wrapper: calls [`assert_status`] and returns the response.
    pub async fn expect_status(
        &self,
        resp: reqwest::Result<Response>,
        expected: StatusCode,
    ) -> Response {
        let resp = resp.expect("request failed");
        let status = resp.status();
        if status != expected {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read body>".to_string());
            panic!(
                "Expected status {}, got {}. Response body: {}",
                expected, status, body
            );
        }
        resp
    }
}

#![deny(unsafe_code)]
/// Main Serve - a high-performance, YAML-configured web server.
///
/// This library crate exposes all modules for use in integration tests.
pub mod config;
/// Database query builders, helpers, and type definitions.
pub mod db;
/// Application error types.
pub mod error;
/// HTTP request handlers for each endpoint action type.
pub mod handlers;
/// Authentication and authorization middleware.
pub mod middleware;
/// HTTP server setup and routing.
pub mod server;
/// Storage backends and file metadata types.
pub mod storage;

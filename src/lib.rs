//! Main Serve - a high-performance, YAML-configured web server.
//!
//! This library crate exposes all modules for use in integration tests.
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::pedantic, clippy::nursery, missing_docs)]

pub mod config;
pub mod db;
pub mod error;
pub mod handlers;
pub mod middleware;
pub mod server;
pub mod storage;

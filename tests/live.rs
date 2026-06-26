//! Live (end-to-end) test harness for Main Serve.
//!
//! These tests spawn the compiled binary as a subprocess and test it via
//! HTTP, exercising the full stack: config loading, database migrations,
//! routing, authentication, and response handling.
//!
//! To run these tests explicitly:
//!
//! ```bash
//! cargo test --features live-tests
//! ```

#![allow(dead_code)]

#[path = "live/support/mod.rs"]
mod support;

#[path = "live/scenarios/mod.rs"]
mod scenarios;

pub use support::*;

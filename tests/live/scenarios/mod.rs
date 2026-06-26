//! Live test scenarios.
//!
//! Each module is a self-contained scenario that exercises Main Serve as a
//! user would: write config, start binary, make HTTP requests, teardown.

pub mod auth_flows;
pub mod headless_blog;
pub mod media_library;
pub mod reverse_proxy;
pub mod spa_host;
pub mod static_files;

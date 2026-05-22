// =============================================================================
// Shared configs
// =============================================================================

pub const MINIMAL_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

endpoints:
  - path: "/ping"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "text/plain"
      body: "pong"
    auth: "none"
"#;

/// Minimal config with a single public custom_response endpoint.
///
/// Equivalent to MINIMAL_CONFIG but with JSON body and no content_type override.
/// Used by tests that only care about a basic working endpoint.
pub const MINIMAL_JSON_CONFIG: &str = r#"
server:
  port: 0

endpoints:
  - path: "/api/public"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"msg": "public"}'
    auth: "none"
"#;

// =============================================================================
// Middleware configs
// =============================================================================

pub const RESPONSE_BODY_LOGGING_CONFIG: &str = r#"
server:
  port: 0

logging:
  log_request_body: false
  log_response_body: true

endpoints:
  - path: "/info"
    methods: ["get"]
    action: custom_response
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"msg":"hello from body logging test"}'
    auth: none
"#;

pub const RESPONSE_BODY_LOGGING_DISABLED_CONFIG: &str = r#"
server:
  port: 0

logging:
  log_request_body: false
  log_response_body: false

endpoints:
  - path: "/ping"
    methods: ["get"]
    action: custom_response
    custom_response:
      status: 200
      body: "pong"
    auth: none
"#;

pub const CORS_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

cors:
  allowed_origins:
    - "https://example.com"
  allowed_methods:
    - "GET"
    - "POST"
  allowed_headers:
    - "Content-Type"
    - "Authorization"
  allow_credentials: true
  max_age: 3600

endpoints:
  - path: "/hello"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"message": "hello"}'
    auth: "none"
"#;

pub const CORS_PER_ENDPOINT_CONFIG: &str = r#"
server:
  port: 0

cors:
  allowed_origins:
    - "https://global.example.com"
  allowed_methods:
    - "GET"

endpoints:
  - path: "/global"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: "global"
    auth: "none"

  - path: "/custom"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: "custom"
    cors:
      allowed_origins:
        - "https://special.example.com"
      allowed_methods:
        - "GET"
        - "POST"
      max_age: 600
    auth: "none"
"#;

pub const RATE_LIMIT_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 0

rate_limit:
  enabled: true
  max_requests: 3
  window_seconds: 60
  key_strategy: header
  key_header: "X-Client-Id"

endpoints:
  - path: "/limited"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"ok": true}'
    auth: "none"
"#;

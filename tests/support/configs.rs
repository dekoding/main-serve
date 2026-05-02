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

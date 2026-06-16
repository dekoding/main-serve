// =============================================================================
// JWT configs
// =============================================================================

pub const JWT_CONFIG: &str = r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-jwt-secret-key-long-enough"
    algorithm: "HS256"
    issuer: "test-issuer"
    audience: "test-audience"
    expiry: 3600
    role_claim: "role"

endpoints:
  - path: "/api/public"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"msg": "public"}'
    auth: "none"

  - path: "/api/private"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"msg": "private"}'
    auth: "jwt"

  - path: "/api/admin"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"msg": "admin only"}'
    auth: "jwt"
    roles: ["admin"]
"#;

pub const JWT_CRUD_CONFIG: &str = r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-jwt-secret-key-long-enough"
    algorithm: "HS256"
    issuer: "test-issuer"
    audience: "test-audience"

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true

tables:
  - name: "__TABLE_NAME__"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "name"
        type: "text"

endpoints:
  - path: "/api/items"
    methods: ["get"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
    auth: "jwt"
"#;

// =============================================================================
// API key configs
// =============================================================================

pub const API_KEY_CONFIG: &str = r#"
server:
  port: 0

auth:
  api_key:
    location: "header"
    name: "X-API-Key"
    keys:
      - key: "secret-api-key-1"
        role: "admin"
      - key: "secret-api-key-2"

endpoints:
  - path: "/api/data"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"data": "secured"}'
    auth: "api_key"

  - path: "/api/admin-data"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"data": "admin"}'
    auth: "api_key"
    roles: ["admin"]
"#;

pub const API_KEY_CRUD_CONFIG: &str = r#"
server:
  port: 0

auth:
  api_key:
    location: "header"
    name: "X-API-Key"
    keys:
      - key: "secret-api-key-1"
        role: "admin"

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true

tables:
  - name: "__TABLE_NAME__"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "name"
        type: "text"

endpoints:
  - path: "/api/items"
    methods: ["get"]
    action: "crud"
    crud:
      table: "__TABLE_NAME__"
      database: "main"
    auth: "api_key"
"#;

pub const API_KEY_QUERY_CONFIG: &str = r#"
server:
  port: 0

auth:
  api_key:
    location: "query"
    name: "api_key"
    keys:
      - key: "secret-query-key"
        role: "admin"

endpoints:
  - path: "/api/data"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"data": "secured"}'
    auth: "api_key"
"#;

// =============================================================================
// OAuth2 configs
// =============================================================================

/// `OAuth2` without JWT config - should be rejected at config load time.
pub const OAUTH2_WITHOUT_JWT_CONFIG: &str = r#"
server:
  port: 0

auth:
  oauth2:
    authorization_url: "https://idp.example.com/authorize"
    token_url: "https://idp.example.com/token"
    client_id: "my-client"
    redirect_url: "http://localhost:8080/_main-serve/oauth2/callback"

endpoints: []
"#;

/// JWT auth via cookie fallback config.
///
/// Tests that `OAuth2` sets a custom cookie name and the JWT middleware
/// also accepts the token from both cookies and Authorization headers.
pub const JWT_COOKIE_FALLBACK_CONFIG: &str = r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-jwt-secret-key-long-enough"
    algorithm: "HS256"
    issuer: "test-issuer"
    audience: "test-audience"
    expiry: 3600
  oauth2:
    cookie_name: "my_auth_cookie"

endpoints:
  - path: "/api/secure"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"msg": "secure"}'
    auth: "jwt"
"#;

// =============================================================================
// Helper functions for OAuth2 configs with placeholders
// =============================================================================

/// Format the `OAuth2` code flow config base with the given `IdP` URL.
pub fn oauth2_code_flow_config(idp_url: &str) -> String {
    format!(
        r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-jwt-secret-key-long-enough"
    algorithm: "HS256"
    issuer: "test-issuer"
    audience: "test-audience"
    expiry: 3600
  oauth2:
    provider: "test-idp"
    authorization_url: "{idp_url}/authorize"
    token_url: "{idp_url}/token"
    userinfo_url: "{idp_url}/userinfo"
    client_id: "test-client-id"
    client_secret: "test-client-secret"
    scopes:
      - "openid"
      - "profile"
    redirect_url: "http://localhost:8080/_main-serve/oauth2/callback"
    success_url: "/dashboard"
    cookie_name: "test_token"

endpoints:
  - path: "/api/protected"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{{"msg": "protected"}}'
    auth: "jwt"
"#,
    )
}

/// Format the `OAuth2` token introspection config base with the given `IdP` URL.
pub fn oauth2_introspection_config(idp_url: &str) -> String {
    format!(
        r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-jwt-secret-key-long-enough"
    algorithm: "HS256"
    issuer: "test-issuer"
    audience: "test-audience"
  oauth2:
    userinfo_url: "{idp_url}/userinfo"
    client_id: "test-client"
    client_secret: "test-secret"
    redirect_url: "http://localhost:8080/_main-serve/oauth2/callback"

endpoints:
  - path: "/api/userdata"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{{"data": "user-specific"}}'
    auth: "oauth2"
"#,
    )
}

// =============================================================================
// Token revocation config
// =============================================================================

pub const JWT_WITH_REVOCATION_CONFIG: &str = r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-jwt-secret-key-long-enough"
    algorithm: "HS256"
    issuer: "test-issuer"
    audience: "test-audience"
    expiry: 3600
    role_claim: "role"
    revocation:
      store: "in_memory"

endpoints:
  - path: "/api/private"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      body: '{"msg": "private"}'
    auth: "jwt"
"#;

// =============================================================================
// Registration config with database
// =============================================================================

pub fn register_config_with_db(template: &str) -> String {
    let rendered = template
        .replace("__DB_DRIVER__", "sqlite")
        .replace("__DB_URL__", "sqlite::memory:")
        .replace("__TABLE_NAME__", "users");
    // Use a unique table name to avoid collisions
    format!(
        "{rendered}\n\ntables:\n  - name: \"users\"\n    database: \"main\"\n    columns:\n      - name: \"id\"\n        type: \"serial\"\n        primary_key: true\n      - name: \"email\"\n        type: \"text\"\n        unique: true\n        nullable: false\n      - name: \"password_hash\"\n        type: \"text\"\n        nullable: false\n      - name: \"role\"\n        type: \"text\"\n        default: \"'user'\"\n      - name: \"created_at\"\n        type: \"timestamptz\"\n        nullable: false\n        default: \"now()\"\n      - name: \"updated_at\"\n        type: \"timestamptz\"\n        nullable: false\n        default: \"now()\"\n"
    )
}

pub const REGISTER_CONFIG_TEMPLATE: &str = r#"
server:
  port: 0

auth:
  jwt:
    secret: "test-jwt-secret-key-long-enough"
    algorithm: "HS256"
    issuer: "main-serve"
    expiry: 3600
    role_claim: "role"
  register:
    enabled: true
    table: "users"
    database: "main"
    default_role: "user"
    password_hash: "argon2id"

databases:
  main:
    driver: "__DB_DRIVER__"
    url: "__DB_URL__"
    auto_migrate: true

endpoints: []
"#;

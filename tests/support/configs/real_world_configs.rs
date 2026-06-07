// =============================================================================
// Real-world scenario configs
// =============================================================================

/// Minimal static file serving config with three modes:
///   - /assets/*   Standard serving (1 day cache)
///   - /app/*      SPA fallback (no cache)
///   - /files/*    Directory listing
///
/// Placeholder: {ROOT} - replaced by `TestDatabase::write_config` or test helpers.
pub const STATIC_FILES_CONFIG: &str = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{ROOT}"

endpoints:
  - path: "/assets/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      index: "index.html"
      directory_listing: false
      cache_max_age: 86400
    auth: "none"

  - path: "/app/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      index: "index.html"
      directory_listing: false
      cache_max_age: 0
    auth: "none"

  - path: "/files/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      index: "index.html"
      directory_listing: true
      cache_max_age: 0
    auth: "none"
"#;

/// Minimal custom responses config covering: JSON API info, health check,
/// HTML landing page, plain text, redirect, 503 maintenance, and XML.
pub const CUSTOM_RESPONSES_CONFIG: &str = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{ROOT}"

endpoints:
  - path: "/api/info"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"name": "My Service", "version": "2.1.0", "environment": "production"}'
      headers:
        X-Service-Name: "my-service"
        X-API-Version: "2.1"
    auth: "none"

  - path: "/health"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"status": "healthy", "uptime": "ok"}'
    auth: "none"

  - path: "/welcome"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "text/html; charset=utf-8"
      body: |
        <!DOCTYPE html>
        <html>
          <head><title>Welcome</title></head>
          <body><h1>Welcome to My Service</h1><p>API docs at /api/info</p></body>
        </html>
    auth: "none"

  - path: "/robots.txt"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "text/plain"
      body: "User-agent: *\nDisallow: /api/\nAllow: /welcome"
    auth: "none"

  - path: "/docs"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 301
      content_type: "text/plain"
      body: "Moved Permanently"
      headers:
        Location: "https://docs.example.com/v2"
        Cache-Control: "public, max-age=86400"
    auth: "none"

  - path: "/api/maintenance"
    methods: ["get", "post", "put", "delete"]
    action: "custom_response"
    custom_response:
      status: 503
      content_type: "application/json"
      body: '{"error": "Service temporarily unavailable", "retry_after": 300}'
      headers:
        Retry-After: "300"
    auth: "none"

  - path: "/api/sitemap.xml"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/xml"
      body: |
        <?xml version="1.0" encoding="UTF-8"?>
        <urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
          <url><loc>https://example.com/</loc></url>
          <url><loc>https://example.com/welcome</loc></url>
        </urlset>
    auth: "none"
"#;

/// Minimal auth-protected API config covering JWT CRUD, API key endpoints,
/// and HTTP Basic admin panel.
///
/// Placeholders:
///   __`DB_DRIVER`__  -> replaced by `TestDatabase`
///   __`DB_URL`__     -> replaced by `TestDatabase`
///   __`TABLE_NAME`__ -> replaced by `TestDatabase`
///   __`BASIC_AUTH_HASH`__ -> replaced by test setup (argon2 hash)
pub const AUTH_PROTECTED_CONFIG: &str = r#"
server:
  port: 0

auth:
  jwt:
    secret: "real-world-test-secret-key-32ch"
    algorithm: "HS256"
    issuer: "main-serve"
    audience: "my-app"
    expiry: 3600
    role_claim: "role"

  api_key:
    location: "header"
    name: "X-API-Key"
    keys:
      - key: "prod-key-admin-001"
        role: "admin"
      - key: "prod-key-reader-002"
        role: "reader"
      - key: "prod-key-writer-003"
        role: "writer"

  basic:
    realm: "My Service Admin"
    users:
      - username: "admin"
        password_hash: "__BASIC_AUTH_HASH__"
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
      - name: "title"
        type: "varchar"
        nullable: false
      - name: "content"
        type: "text"
        nullable: true
      - name: "status"
        type: "varchar"
        nullable: false
        default: "'draft'"
      - name: "created_at"
        type: "timestamp"
        nullable: false
        default: "CURRENT_TIMESTAMP"

stores:
  local_assets:
    backend: native
    root: "{ROOT}"

endpoints:
  - path: "/health"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"status": "ok"}'
    auth: "none"

  - path: "/api/articles"
    methods: ["get", "post"]
    action: "crud"
    crud:
      database: "main"
      table: "__TABLE_NAME__"
      fields: ["id", "title", "content", "status", "created_at"]
      writable_fields: ["title", "content", "status"]
      pagination:
        enabled: true
        default_page_size: 20
        max_page_size: 100
      filtering:
        enabled: true
        allowed_fields: ["status"]
      sorting:
        enabled: true
        default_field: "created_at"
        default_order: "desc"
    auth: "jwt"

  - path: "/api/articles/{id}"
    methods: ["get", "put", "delete"]
    action: "crud"
    crud:
      database: "main"
      table: "__TABLE_NAME__"
      fields: ["id", "title", "content", "status", "created_at"]
      writable_fields: ["title", "content", "status"]
    auth: "jwt"
    roles: ["admin"]

  - path: "/api/feed"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"feed": "articles", "format": "json"}'
    auth: "api_key"
    roles: ["reader", "admin"]

  - path: "/api/admin/stats"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"total_articles": 42, "active_users": 7}'
    auth: "api_key"
    roles: ["admin"]

  - path: "/admin/config"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"config": "current settings"}'
    auth: "basic"
    roles: ["admin"]
"#;

/// Minimal combined config: SPA frontend + health check + JWT CRUD users +
/// API key service endpoint + HTTP Basic admin panel.
///
/// Placeholders:
///   {ROOT}              -> replaced by test helpers
///   __`DB_DRIVER`__       -> replaced by `TestDatabase`
///   __`DB_URL`__          -> replaced by `TestDatabase`
///   __`TABLE_NAME`__      -> replaced by `TestDatabase`
///   __`BASIC_AUTH_HASH`__ -> replaced by test setup (argon2 hash)
pub const COMBINED_CONFIG: &str = r#"
server:
  port: 0

auth:
  jwt:
    secret: "real-world-test-secret-key-32ch"
    algorithm: "HS256"
    issuer: "main-serve"
    audience: "my-app"
    expiry: 3600
    role_claim: "role"

  api_key:
    location: "header"
    name: "X-API-Key"
    keys:
      - key: "service-to-service-key"
        role: "service"

  basic:
    realm: "Admin Panel"
    users:
      - username: "admin"
        password_hash: "__BASIC_AUTH_HASH__"
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
        type: "varchar"
        nullable: false
      - name: "email"
        type: "varchar"
        nullable: false
        unique: true
      - name: "role"
        type: "varchar"
        nullable: false
        default: "'user'"
      - name: "active"
        type: "boolean"
        nullable: false
        default: "true"
      - name: "created_at"
        type: "timestamp"
        nullable: false
        default: "CURRENT_TIMESTAMP"

stores:
  local_assets:
    backend: native
    root: "{ROOT}"

endpoints:
  - path: "/app/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      index: "index.html"
      cache_max_age: 0
    auth: "none"

  - path: "/assets/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      index: "index.html"
      cache_max_age: 604800
    auth: "none"

  - path: "/health"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"status": "healthy", "version": "1.0.0"}'
    auth: "none"

  - path: "/api/users"
    methods: ["get", "post"]
    action: "crud"
    crud:
      database: "main"
      table: "__TABLE_NAME__"
      fields: ["id", "name", "email", "role", "active", "created_at"]
      writable_fields: ["name", "email", "role", "active"]
      pagination:
        enabled: true
        default_page_size: 25
        max_page_size: 100
      filtering:
        enabled: true
        allowed_fields: ["role", "active"]
      sorting:
        enabled: true
        default_field: "created_at"
        default_order: "desc"
      where_clause: "active = true"
    auth: "jwt"

  - path: "/api/users/{id}"
    methods: ["get", "put", "delete"]
    action: "crud"
    crud:
      database: "main"
      table: "__TABLE_NAME__"
      fields: ["id", "name", "email", "role", "active", "created_at"]
      writable_fields: ["name", "email", "role", "active"]
    auth: "jwt"
    roles: ["admin"]

  - path: "/api/service/ping"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"pong": true}'
    auth: "api_key"
    roles: ["service"]

  - path: "/admin/status"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"admin": true, "panel": "status"}'
    auth: "basic"
    roles: ["admin"]
"#;

/// Minimal `OAuth2` config with token introspection, role-restricted endpoint,
/// and JWT-protected dashboard.
///
/// Placeholder:
///   __`IDP_URL`__ -> replaced by test helpers with mock `IdP` URL
pub const OAUTH2_CONFIG: &str = r#"
server:
  port: 0

auth:
  jwt:
    secret: "oauth2-test-jwt-secret-32chars!"
    algorithm: "HS256"
    issuer: "main-serve"
    audience: "my-app"
    expiry: 3600
    role_claim: "role"

  oauth2:
    provider: "test-idp"
    authorization_url: "__IDP_URL__/authorize"
    token_url: "__IDP_URL__/token"
    userinfo_url: "__IDP_URL__/userinfo"
    client_id: "my-app-client-id"
    client_secret: "my-app-client-secret"
    scopes: ["openid", "profile", "email"]
    redirect_url: "http://localhost:8080/_main-serve/oauth2/callback"
    success_url: "/dashboard"
    cookie_name: "app_session"
    state_ttl: 300
    max_pending_states: 100

stores:
  local_assets:
    backend: native
    root: "{ROOT}"

endpoints:
  - path: "/welcome"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "text/html; charset=utf-8"
      body: |
        <!DOCTYPE html>
        <html>
          <head><title>Welcome</title></head>
          <body>
            <h1>Welcome</h1>
            <a href="/_main-serve/oauth2/authorize">Sign in with SSO</a>
          </body>
        </html>
    auth: "none"

  - path: "/api/profile"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"resource": "user profile"}'
    auth: "oauth2"

  - path: "/api/admin/settings"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"admin": true, "settings": "all"}'
    auth: "oauth2"
    roles: ["admin"]

  - path: "/api/dashboard"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"page": "dashboard", "widgets": ["activity", "stats"]}'
    auth: "jwt"
"#;

/// Minimal reverse proxy config with three proxy routes:
///   - /api/v1/*  -> path rewrite (strip /api/v1, add /v1) + custom headers
///   - /legacy/*  -> legacy migration rewrite
///   - /external/time -> passthrough
///
/// Placeholder:
///   __`UPSTREAM_URL`__ -> replaced by test helpers with mock upstream URL
pub const PROXY_CONFIG: &str = r#"
server:
  port: 0

stores:
  local_assets:
    backend: native
    root: "{ROOT}"

endpoints:
  - path: "/api/v1/*"
    methods: ["get", "post", "put", "delete"]
    action: "proxy"
    proxy:
      upstream: "__UPSTREAM_URL__"
      path_rewrite:
        strip_prefix: "/api/v1"
        add_prefix: "/v1"
      headers:
        X-Forwarded-By: "main-serve"
        X-Request-Source: "gateway"
      timeouts:
        connect: 5
        read: 30
        total: 60
      max_response_size: 10485760
    auth: "none"

  - path: "/legacy/*"
    methods: ["get"]
    action: "proxy"
    proxy:
      upstream: "__UPSTREAM_URL__"
      path_rewrite:
        strip_prefix: "/legacy"
        add_prefix: "/api/v2"
      timeouts:
        connect: 3
        read: 15
        total: 20
    auth: "none"

  - path: "/external/time"
    methods: ["get"]
    action: "proxy"
    proxy:
      upstream: "__UPSTREAM_URL__"
      timeouts:
        connect: 2
        read: 5
        total: 8
    auth: "none"
"#;

/// Minimal CRUD API with multiple related tables, joins, computed fields,
/// and API key auth with three roles.
///
/// Placeholders:
///   __`DB_DRIVER`__  -> replaced by `TestDatabase`
///   __`DB_URL`__     -> replaced by `TestDatabase`
///   __`TABLE_NAME`__ -> replaced by `TestDatabase` (related tables suffixed)
pub const CRUD_API_CONFIG: &str = r#"
server:
  port: 0

auth:
  api_key:
    location: "header"
    name: "X-API-Key"
    keys:
      - key: "read-key-001"
        role: "reader"
      - key: "write-key-002"
        role: "writer"
      - key: "admin-key-003"
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
      - name: "title"
        type: "varchar"
        nullable: false
        indexed: true
      - name: "slug"
        type: "varchar"
        nullable: false
        unique: true
      - name: "body"
        type: "text"
        nullable: false
      - name: "status"
        type: "varchar"
        nullable: false
        default: "'draft'"
      - name: "author_id"
        type: "integer"
        nullable: false
      - name: "category_id"
        type: "integer"
        nullable: true
      - name: "view_count"
        type: "integer"
        nullable: false
        default: "0"
      - name: "created_at"
        type: "timestamp"
        nullable: false
        default: "CURRENT_TIMESTAMP"
      - name: "published_at"
        type: "timestamp"
        nullable: true
    foreign_keys:
      - column: "author_id"
        references_table: "__TABLE_NAME___authors"
        references_column: "id"
        on_delete: "cascade"
      - column: "category_id"
        references_table: "__TABLE_NAME___categories"
        references_column: "id"
        on_delete: "set_null"

  - name: "__TABLE_NAME___authors"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "name"
        type: "varchar"
        nullable: false
      - name: "email"
        type: "varchar"
        nullable: false
        unique: true
      - name: "bio"
        type: "text"
        nullable: true
      - name: "created_at"
        type: "timestamp"
        nullable: false
        default: "CURRENT_TIMESTAMP"

  - name: "__TABLE_NAME___categories"
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "name"
        type: "varchar"
        nullable: false
        unique: true
      - name: "description"
        type: "text"
        nullable: true

stores:
  local_assets:
    backend: native
    root: "{ROOT}"

endpoints:
  - path: "/api/authors"
    methods: ["get", "post"]
    action: "crud"
    crud:
      database: "main"
      table: "__TABLE_NAME___authors"
      fields: ["id", "name", "email", "created_at"]
      writable_fields: ["name", "email", "bio"]
      pagination:
        enabled: true
        default_page_size: 20
        max_page_size: 50
      sorting:
        enabled: true
        default_field: "name"
        default_order: "asc"
        allowed_fields: ["id", "name", "created_at"]
      filtering:
        enabled: true
        allowed_fields: ["name"]
    auth: "none"

  - path: "/api/authors/:id"
    methods: ["get", "put", "delete"]
    action: "crud"
    crud:
      database: "main"
      table: "__TABLE_NAME___authors"
      fields: ["id", "name", "email", "bio", "created_at"]
      writable_fields: ["name", "email", "bio"]
    auth: "api_key"
    roles: ["writer", "admin"]

  - path: "/api/categories"
    methods: ["get", "post"]
    action: "crud"
    crud:
      database: "main"
      table: "__TABLE_NAME___categories"
      fields: ["id", "name", "description"]
      writable_fields: ["name", "description"]
      pagination:
        enabled: true
        default_page_size: 50
        max_page_size: 100
      sorting:
        enabled: true
        default_field: "name"
        default_order: "asc"
    auth: "none"

  - path: "/api/categories/:id"
    methods: ["get", "put", "delete"]
    action: "crud"
    crud:
      database: "main"
      table: "__TABLE_NAME___categories"
      fields: ["id", "name", "description"]
      writable_fields: ["name", "description"]
    auth: "api_key"
    roles: ["writer", "admin"]

  - path: "/api/articles"
    methods: ["get"]
    action: "crud"
    crud:
      database: "main"
      table: "__TABLE_NAME__"
      fields:
        - "id"
        - "title"
        - "slug"
        - "status"
        - "view_count"
        - "created_at"
        - "published_at"
      pagination:
        enabled: true
        default_page_size: 20
        max_page_size: 100
      sorting:
        enabled: true
        default_field: "created_at"
        default_order: "desc"
        allowed_fields:
          - "id"
          - "title"
          - "view_count"
          - "created_at"
          - "published_at"
      filtering:
        enabled: true
        allowed_fields: ["status", "author_id", "category_id"]
      where_clause: "__TABLE_NAME__.status = 'published'"
      joins:
        - table: "__TABLE_NAME___authors"
          on: "__TABLE_NAME__.author_id = __TABLE_NAME___authors.id"
          join_type: "inner"
          fields:
            - "__TABLE_NAME___authors.name AS author_name"
        - table: "__TABLE_NAME___categories"
          on: "__TABLE_NAME__.category_id = __TABLE_NAME___categories.id"
          join_type: "left"
          fields:
            - "__TABLE_NAME___categories.name AS category_name"
      computed_fields:
        - name: "body_length"
          expression: "LENGTH(__TABLE_NAME__.body)"
    auth: "none"

  - path: "/api/articles"
    methods: ["post"]
    action: "crud"
    crud:
      database: "main"
      table: "__TABLE_NAME__"
      writable_fields:
        - "title"
        - "slug"
        - "body"
        - "status"
        - "author_id"
        - "category_id"
    auth: "api_key"
    roles: ["writer", "admin"]

  - path: "/api/articles/:id"
    methods: ["get"]
    action: "crud"
    crud:
      database: "main"
      table: "__TABLE_NAME__"
      fields:
        - "id"
        - "title"
        - "slug"
        - "body"
        - "status"
        - "author_id"
        - "category_id"
        - "view_count"
        - "created_at"
        - "published_at"
    auth: "none"

  - path: "/api/articles/:id"
    methods: ["put"]
    action: "crud"
    crud:
      database: "main"
      table: "__TABLE_NAME__"
      writable_fields:
        - "title"
        - "slug"
        - "body"
        - "status"
        - "category_id"
        - "view_count"
    auth: "api_key"
    roles: ["writer", "admin"]

  - path: "/api/articles/:id"
    methods: ["delete"]
    action: "crud"
    crud:
      database: "main"
      table: "__TABLE_NAME__"
    auth: "api_key"
    roles: ["admin"]
"#;

/// Format the `OAuth2` config with the given `IdP` URL.
pub fn oauth2_config(idp_url: &str) -> String {
    OAUTH2_CONFIG.replace("__IDP_URL__", idp_url)
}

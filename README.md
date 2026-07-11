# Main Serve

Never write boilerplate CRUD endpoints again! **Main Serve** is an extremely high-performance, YAML-configured web server written in Rust. It takes all the boilerplate code you hate writing and replaces it with pure and simple configuration. Define API endpoints, database schemas, authentication, proxying, static file serving, CORS, and more - all in a single YAML file! No code required.

## Features

- **Pure YAML configuration** - No code outside YAML to define endpoints. It's all done in config.
- **CRUD endpoints** - Create auto-generated REST APIs directly from table schemas (SQLite, PostgreSQL, MySQL).
- **Auto-migration** - All table schemas defined in the YAML config are created/updated automatically.
- **Authentication** - Supports JWT, API key, HTTP Basic, and even OAuth2/OIDC!
- **Role-based authorization** - Define per-endpoint role restrictions.
- **Reverse proxy** - Implement upstream forwarding with path rewriting and header injection.
- **Static file serving** - Directory/HTML serving with SPA fallback support for frontend routing.
- **Static file management** - Upload files and retrieve images in specified resolutions.
- **CORS** - Set global and per-endpoint CORS policies.
- **Rate limiting** - Configurable per-endpoint or globally, keyed by IP/header/token.
- **Hot reload** - Update config without restarting the server or dropping connections.
- **TLS** - Built-in HTTPS support via rustls (no OpenSSL dependency).
- **Request tracing** - Unique request IDs, structured logging, gzip compression.
- **Graceful shutdown** - Drain all in-flight requests on SIGTERM/SIGINT.

## Quickstart

### 1. Build

```bash
cargo build --release
```

### 2. Create a config file

Create `config.yaml`:

```yaml
server:
  host: "127.0.0.1"
  port: 8080

databases:
  main:
    driver: "sqlite"
    url: "sqlite://app.db?mode=rwc"
    max_connections: 5
    auto_migrate: true

tables:
  - name: "todos"
    database: "main"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true
      - name: "title"
        type: "varchar"
        nullable: false
      - name: "done"
        type: "boolean"
        default: "false"

endpoints:
  - path: "/api/todos"
    methods: ["get", "post"]
    action: "crud"
    crud:
      table: "todos"
      database: "main"
      fields: ["id", "title", "done"]
      writable_fields: ["title", "done"]
    auth: "none"

  - path: "/api/todos/{id}"
    methods: ["get", "put", "delete"]
    action: "crud"
    crud:
      table: "todos"
      database: "main"
      fields: ["id", "title", "done"]
      writable_fields: ["title", "done"]
    auth: "none"
```

### 3. Run

```bash
MAIN_SERVE_ADMIN_TOKEN=my-secret ./target/release/main-serve -c config.yaml
```

### 4. Use

```bash
# Create a todo
curl -X POST http://localhost:8080/api/todos \
  -H "Content-Type: application/json" \
  -d '{"title": "Learn Rust", "done": false}'

# List all todos
curl http://localhost:8080/api/todos

# Health check
curl http://localhost:8080/_main-serve/health
```

## CLI

```
Usage: main-serve [OPTIONS]

Options:
  -c, --config <CONFIG>
          Path to YAML config file.
          
          If not specified, checks `$HOME/.config/main-serve/config.yaml` then `/etc/main-serve/config.yaml`.

      --admin-token <ADMIN_TOKEN>
          Admin token for the reload endpoint (overrides `MAIN_SERVE_ADMIN_TOKEN` env var)
          
          [env: MAIN_SERVE_ADMIN_TOKEN=]
          [default: ""]

      --validate
          Validate config and exit without starting the server

      --dry-run
          Parse config, print resolved endpoints, and exit

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

## Configuration Reference

See the full [Configuration Reference](docs/CONFIGURATION.md) for detailed documentation of every field, type, default, and option.

For the annotated YAML specification, see [config/spec.yaml](config/spec.yaml).

### Server

```yaml
server:
  host: "127.0.0.1"             # Bind address
  port: 8080                    # Bind port
  workers: 0                    # Worker threads (0 = auto)
  max_body_size: 10485760       # Max request body (bytes)
  keep_alive: 75                # Keep-alive timeout in seconds (0 = disabled)
  shutdown_timeout: 30          # Graceful shutdown timeout (seconds)
  tls:                          # Optional - omit for plain HTTP
    cert: "/path/to/cert.pem"
    key: "/path/to/key.pem"
```

### Databases

```yaml
databases:
  my_db:
    driver: "sqlite"             # sqlite, postgres, mysql
    url: "sqlite://data.db?mode=rwc"
    min_connections: 1
    max_connections: 10
    auto_migrate: true           # Auto-create tables on startup
    allow_destructive: false     # Allow dropping columns missing from YAML
    acquire_timeout: 5           # Pool connection acquire timeout (seconds)
```

### Logging

```yaml
logging:
  level: "info"                  # trace, debug, info, warn, error
  format: "pretty"               # pretty or json
  log_request_body: false        # Log request bodies (debug use)
  log_response_body: false       # Log response bodies (debug use)
```

### Table Schemas

The `tables` section is an array where each entry defines a table:

```yaml
tables:
  - name: "users"
    database: "my_db"
    columns:
      - name: "id"
        type: "integer"
        primary_key: true
      - name: "email"
        type: "varchar"
        nullable: false
        unique: true
      - name: "name"
        type: "varchar"
    foreign_keys: []
```

Supported column types: `integer`, `bigint`, `smallint`, `serial`, `bigserial`, `varchar`, `char`, `text`, `boolean`, `float`, `double`, `decimal`, `date`, `timestamp`, `timestamptz`, `uuid`, `json`, `jsonb`, `bytea`, `blob`.

### Endpoints

#### CRUD

```yaml
endpoints:
  - path: "/api/users"
    methods: ["get", "post"]
    action: "crud"
    crud:
      table: "users"
      database: "main"
      fields: ["id", "email", "name"]
      writable_fields: ["email", "name"]
      where_clause: "active = true"
      insert_owner: "created_by"
      computed_fields:
        - name: "full_name"
          expression: "first_name || ' ' || last_name"
      pagination:
        enabled: true
        default_page_size: 20
        max_page_size: 100
      sorting:
        enabled: true
        default_field: "id"
        default_order: "asc"
      filtering:
        enabled: true
        allowed_fields: ["email"]
    auth: "none"
```

The CRUD action also supports `update_where_clause` and `delete_where_clause` for scoped updates and deletions, and `joins` for including data from related tables. All WHERE clauses support request context interpolation (e.g., `"author_id = ${request.user.id}"`).

#### Reverse Proxy

```yaml
  - path: "/upstream/*"
    methods: ["get", "post", "put", "delete"]
    action: "proxy"
    proxy:
      upstream: "http://backend:3000"
      path_rewrite:
        strip_prefix: "/upstream"
        add_prefix: ""          # Optional prefix to add after stripping
      timeouts:
        connect: 5              # Connection timeout (seconds)
        read: 30                # Read timeout (seconds)
        total: 60               # Total request timeout (seconds)
      headers:
        X-Forwarded-By: "main-serve"
    auth: "none"
```

#### Storage Backends

Named storage targets for file serving and media. Endpoints reference these by name.

```yaml
stores:
  local_assets:
    backend: "native"
    root: "./public"
```

#### Static Files

```yaml
stores:
  local_assets:
    backend: "native"
    root: "./public"

endpoints:
  - path: "/static/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "local_assets"
      index: "index.html"
      directory_listing: false
      cache_max_age: 3600
      etag: true
      range_requests: true
      head_support: true
      upload:
        enabled: true
        max_size: 10485760
        allowed_extensions:
          - ".jpg"
          - ".jpeg"
          - ".png"
          - ".gif"
          - ".pdf"
        create_subdirectory: "{user_id}/{year}/{month}"
      image_resize:
        enabled: true
        max_dimension: 4096
        supported_formats:
          - "jpg"
          - "jpeg"
          - "png"
          - "webp"
        default_fit: "scale_down"
      streaming:
        enabled: true
        buffer_size: 65536
        threshold: 1048576
        include_content_length: true
    auth: "none"
```

#### Media Library

Full media management with uploads, trash, sharing, and image resizing. Requires a database table for metadata and a storage backend.

```yaml
- path: "/media"
  methods: ["get", "post", "patch", "delete"]
  action: "media"
  media:
    storage: "media_storage"
    table: "media_items"
    database: "main"
    columns: ["auto", "tags"]
    trash:
      enabled: true
      retention_days: 14
    sharing:
      enabled: true
      signing_secret: "${FILE_SHARING_SECRET}"
    image_resize:
      enabled: true
      max_dimension: 2048
```

#### File Store

Database-backed file catalog with ownership tracking. File serving requires a separate `static_files` endpoint.

```yaml
- path: "/api/files"
  methods: ["get", "post", "patch", "delete"]
  action: "file_store"
  file_store:
    storage: "media_storage"
    table: "files"
    database: "main"
    ownership:
      owner_column: "uploader_id"
      admin_override: true
```

#### SPA Host

Serve a single-page application with automatic fallback routing.

```yaml
- path: "/app/*"
  methods: ["get", "head"]
  action: "spa_host"
  spa_host:
    storage: "local_assets"
    index: "index.html"
    cache_max_age: 3600
    fallback_status: 200
```

#### Custom Response

```yaml
  - path: "/api/status"
    methods: ["get"]
    action: "custom_response"
    custom_response:
      status: 200
      content_type: "application/json"
      body: '{"status": "ok"}'
    auth: "none"
```

### Authentication

```yaml
auth:
  jwt:
    secret: "${JWT_SECRET}"     # Use env vars for secrets
    algorithm: "HS256"
    issuer: "my-app"
    audience: "my-api"
    expiry: 3600
    role_claim: "role"          # JWT claim containing user role
  api_key:
    location: "header"          # header or query
    name: "X-API-Key"
    keys:
      - key: "${API_KEY}"
        role: "admin"
  basic:
    realm: "my-app"              # WWW-Authenticate realm
    users:
      - username: "admin"
        password_hash: "$argon2id$..."  # Argon2 hash
        role: "admin"
```

Per-endpoint auth is set via the `auth` field (`"jwt"`, `"api_key"`, `"basic"`, `"oauth2"`, `"none"`).

#### OAuth2/OIDC

Full authorization code flow with PKCE and token introspection support. Requires `auth.jwt` for the code flow.

```yaml
auth:
  jwt:
    secret: "${JWT_SECRET}"     # Required for code flow (mints JWTs)
  oauth2:
    provider: "generic"          # Provider name (for logging)
    authorization_url: "https://provider.com/authorize"
    token_url: "https://provider.com/token"
    userinfo_url: "https://provider.com/userinfo"
    client_id: "${OAUTH_CLIENT_ID}"
    client_secret: "${OAUTH_CLIENT_SECRET}"
    scopes: ["openid", "profile", "email"]
    redirect_url: "http://localhost:8080/_main-serve/oauth2/callback"
    success_url: "/"             # Where to redirect after login
    cookie_name: "main_serve_token"  # HttpOnly cookie for JWT
    state_ttl: 300               # Pending state lifetime (seconds)
    max_pending_states: 1000     # Max concurrent authorization flows
    role_mapping:
      default_role: "user"
      role_claim: "groups"
      role_map:
        "admins": "admin"
        "editors": "editor"
      match_mode: "exact"       # exact or contains
```

#### User Registration

Enable self-service registration:

```yaml
auth:
  register:
    enabled: true
    table: "users"
    database: "main"
    default_role: "user"
    password_hash: "argon2id"
```

### CORS

```yaml
cors:
  allowed_origins: ["https://example.com"]
  allowed_methods: ["GET", "POST", "PUT", "DELETE"]
  allowed_headers: ["Content-Type", "Authorization"]
  allow_credentials: true
  max_age: 86400
```

### Rate Limiting

```yaml
rate_limit:
  enabled: true
  max_requests: 100
  window_seconds: 60
  key_strategy: "ip"            # ip, header, token
  key_header: ""                # Used when key_strategy is "header"
  cleanup_threshold: 10000      # Clean up expired entries when threshold reached
```

### Environment Variables

YAML values support `${ENV_VAR}` and `${ENV_VAR:-default}` syntax for injecting secrets:

```yaml
databases:
  prod:
    url: "${DATABASE_URL}"
auth:
  jwt:
    secret: "${JWT_SECRET:-dev-secret}"
```

### File Includes

Large configs can be split across multiple files using the `$include` directive. Paths are relative to the file containing the directive. Glob patterns are supported.

```yaml
# In a sequence (e.g. endpoints):
endpoints:
  - $include: "endpoints/*.yaml"

# In a mapping (e.g. databases):
databases:
  $include: "db/*.yaml"
```

Includes can be nested. Circular includes are detected and rejected.

## Hot Reload

Update your YAML config and reload without downtime:

```bash
curl -X POST http://localhost:8080/_main-serve/reload \
  -H "Authorization: Bearer $MAIN_SERVE_ADMIN_TOKEN"
```

- In-flight requests complete against the old config
- New requests use the new config immediately
- TCP listeners stay open - no connection drops
- Invalid configs are rejected without affecting the running server

## System Endpoints

| Endpoint | Method | Auth | Description |
|---|---|---|---|
| `/_main-serve/health` | GET | None | Health check with DB status |
| `/_main-serve/reload` | POST | Bearer token | Hot-reload configuration |
| `/_main-serve/oauth2/authorize` | GET | None | Start OAuth2 login flow (if configured) |
| `/_main-serve/oauth2/callback` | GET | None | OAuth2 IdP callback (if configured) |

## License

MIT

## Documentation

- [Quickstart Guide](docs/QUICKSTART.md) - Get running in minutes
- [Configuration Reference](docs/CONFIGURATION.md) - Every field explained
- [YAML Spec](config/spec.yaml) - Annotated configuration specification
- [Manifesto](docs/MANIFESTO.md) - Why Main Serve exists
- [Contributing](CONTRIBUTING.md) - How to contribute

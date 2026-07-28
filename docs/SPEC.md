# Main Serve Behavioral Specification

This document describes how Main Serve works -- its runtime behavior, configuration semantics, and the interactions between its subsystems. The canonical JSON Schema for configuration structure is at `config/main-serve.schema.json` and the example configuration is at `config/templates/example.yaml`.

## 1. Overview

Main Serve is a no-code, YAML-configured HTTP API server written in Rust. It eliminates boilerplate for common web application backends: database CRUD APIs, static file hosting, media libraries, reverse proxying, and single-page application serving -- all driven by a single `config.yaml` file.

**Tech stack**

- Language: Rust (edition 2024, MSRV pinned in `rust-toolchain.toml`)
- HTTP framework: axum 0.8
- Database driver: sqlx 0.8
- Async runtime: tokio (multi-thread for servers, current_thread for CLIs)
- Serialization: serde + serde_yaml
- Logging: tracing

**How the configuration file fits in**

1. The user writes `config.yaml` (or any path, see Section 2) describing servers, databases, tables, auth providers, storage backends, and endpoints.
2. At startup (and on hot reload), the config is loaded, env vars are interpolated, `$include` directives are resolved, YAML is parsed, structs are deserialized, and semantic validation runs.
3. On success, the server spins up with all configured endpoints registered against the axum router.

## 2. Configuration Loading

### 2.1 Config File Discovery Order

The config file is resolved in this order (first match wins):

1. CLI flag: `--config <path>` or `-c <path>`
2. `$HOME/.config/main-serve/config.yaml`
3. `/etc/main-serve/config.yaml`

The path is canonicalized before any processing to prevent path traversal issues.

### 2.2 Environment Variable Interpolation

Before YAML parsing, every string in the raw file is scanned for the patterns `${VAR}` and `${VAR:-default}`.

| Pattern | Behavior |
|---------|----------|
| `${VAR}` | Replaced with `$VAR`. Fails config load if `VAR` is unset. |
| `${VAR:-fallback}` | Replaced with `$VAR` if set, otherwise `fallback`. |

The regex pattern is `\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-((?:[^}])*))?\}`. Multiple substitutions in a single string are all resolved. If any variable is unset without a default, the error message lists all offending variables:

```
Environment variable interpolation failed:
  - Environment variable 'DB_PASSWORD' is not set and has no default
  - Environment variable 'API_KEY' is not set and has no default
```

### 2.3 File Includes (`$include` Directive)

The `$include` directive enables modular config files. Paths are resolved relative to the file containing the directive.

**Sequence context** (e.g., `endpoints:`):

```yaml
endpoints:
  - $include: "endpoints/users.yaml"         # single file, replaces item
  - $include: "endpoints/*.yaml"             # glob, one item per match
  - path: "/inline"                           # inline entries still work
    action: custom_response
```

Each matching file is loaded and its top-level value is inserted at that position in the sequence. Glob matches are sorted for deterministic ordering.

**Mapping context** (e.g., `tables:`):

```yaml
tables:
  $include: "tables/*.yaml"   # merges all matches into one mapping
```

If the include is the only key in the mapping, it replaces the mapping entirely. With a glob matching multiple files, all results are merged (later files overwrite earlier keys on collision).

**Collision precedence**: When two included files define the same key (e.g., two tables named "users"), the last-included file takes precedence.

**Circular include detection**: The loader tracks visited canonical paths in a `HashSet<PathBuf>`. If a file is encountered that is already in the visited set, config loading fails with:

```
Circular $include detected: /path/to/file.yaml was already included
```

Nested `$include` directives in included files are also supported, with env var interpolation applied to them as well.

### 2.4 Loading Pipeline

The full config load pipeline is:

1. **Read** the YAML file from disk (canonicalized path).
2. **Interpolate** `${VAR}` / `${VAR:-default}` patterns in raw text.
3. **Parse** the interpolated text into a `serde_yaml::Value` tree.
4. **Resolve** `$include` directives recursively.
5. **Deserialize** into strongly-typed `AppConfig` structs.
6. **Validate** semantics (cross-references, required fields, safe SQL fragments, etc.).

Steps 2-4 may fail with `AppError::Config`. Step 6 may fail with `AppError::Validation`. The complete validation error is combined into a single message:

```
Configuration validation failed:
  - databases.main.url must not be empty
  - tables[0] (users): references database 'main' which is not defined in databases
  - endpoints[2] (/api/users): auth 'jwt' is not a configured auth provider
```

## 3. Server Settings

### 3.1 Bind Address and Port

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `host` | string | `"127.0.0.1"` | Listen address. Use `"0.0.0.0"` for all interfaces. |
| `port` | integer (1-65535) | `8080` | TCP port. |

### 3.2 Worker Threads

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `workers` | integer | `0` (auto) | Tokio worker threads. `0` = number of CPUs. |

### 3.3 Body and Connection Limits

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `max_body_size` | integer (bytes) | `10485760` (10 MiB) | Max request body. Exceeding this returns HTTP 413. Does not apply to file uploads or proxy requests. |
| `keep_alive` | integer (seconds) | `75` | HTTP keep-alive idle timeout. `0` disables keep-alive. Implemented via hyper's `header_read_timeout`. |
| `shutdown_timeout` | integer (seconds) | `30` | Graceful shutdown window after SIGTERM/SIGINT. |

### 3.4 TLS

```yaml
server:
  tls:
    cert: "/path/to/cert.pem"
    key: "/path/to/key.pem"
```

Omit the `tls` section entirely for plain HTTP. When present, both `cert` and `key` are required. Validation enforces:

```
server.tls.cert must not be empty when TLS is configured
server.tls.key must not be empty when TLS is configured
```

The TLS layer is handled by hyper/tokio-rustls. Key material is redacted from debug output.

### 3.5 Graceful Shutdown

On SIGTERM or SIGINT, Main Serve stops accepting new connections and gives in-flight requests `shutdown_timeout` seconds to complete. Tasks that exceed this timeout are aborted. The shutdown is atomic -- no partial config or pool state is left behind.

## 4. Logging

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `level` | enum | `"info"` | Log verbosity: `trace`, `debug`, `info`, `warn`, `error`. |
| `format` | enum | `"pretty"` | Output format: `json` or `pretty`. |
| `log_request_body` | boolean | `false` | Buffer request body in memory, log at DEBUG level, replay to handler. |
| `log_response_body` | boolean | `false` | Buffer response body in memory, log at DEBUG level, forward to client. |
| `max_body_log_size` | integer (bytes) | `16384` (16 KiB) | Maximum body bytes included in logs. Larger bodies are truncated. |

**Memory overhead**: Enabling `log_request_body` or `log_response_body` buffers the entire body in memory for every request, even if it is later truncated for logging. This adds per-request memory overhead proportional to the request body size. Only enable in development or for specific debugging sessions.

## 5. CORS

Global CORS defaults apply to all endpoints unless overridden per-endpoint.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `allowed_origins` | list of strings | `[]` | Allowed origins. `["*"]` for all. Empty = no cross-origin access. |
| `allowed_methods` | list of strings | all methods | HTTP methods allowed. Default includes GET, POST, PUT, PATCH, DELETE, OPTIONS, HEAD. |
| `allowed_headers` | list of strings | `[]` | Allowed request headers. |
| `allow_credentials` | boolean | `false` | Includes `Access-Control-Allow-Credentials` header. |
| `max_age` | integer (seconds) | `86400` (24h) | Preflight response cache duration. |

**Per-endpoint override**: Set `cors` on an endpoint to override the global policy. Omit to inherit. The per-endpoint config has the same shape as the global config.

**OPTIONS handling**: When CORS is configured (globally or per-endpoint), the server automatically handles `OPTIONS` preflight requests, returning the appropriate `Access-Control-*` headers. The endpoint does not need to explicitly list `options` in its `methods`.

## 6. Rate Limiting

Global rate limit defaults apply to all endpoints unless overridden per-endpoint.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | boolean | `false` | Activate rate limiting. |
| `max_requests` | integer | `100` | Maximum requests per window. |
| `window_seconds` | integer | `60` | Sliding window duration in seconds. |
| `key_strategy` | enum | `"ip"` | Key extraction strategy: `ip`, `header`, `token`. |
| `key_header` | string | `""` | Header name when `key_strategy` is `header`. |
| `cleanup_threshold` | integer | `10000` | Trigger cleanup of expired entries when this many tracked clients exist. |

**Key strategies**:

- `ip`: Rate limit by client IP address (from remote_addr or forwarded headers).
- `header`: Rate limit by the value of a specific header (set via `key_header`).
- `token`: Rate limit by authenticated user's API key or JWT token.

**Cleanup**: When the number of tracked client entries exceeds `cleanup_threshold`, expired entries (past the sliding window) are evicted from memory. This prevents unbounded memory growth in high-traffic deployments.

**Per-endpoint override**: Set `rate_limit` on an endpoint to override the global policy. Omit to inherit.

## 7. Databases

### 7.1 Named Connections

Databases are defined as a named map. Each key is referenced by endpoints via `database: <name>`.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `driver` | enum | required | Database backend: `postgres`, `mysql`, `sqlite`. |
| `url` | string | required | Connection string (use `${ENV_VAR}` for secrets). |
| `min_connections` | integer | `1` | Minimum connections in the pool. |
| `max_connections` | integer | `10` | Maximum connections in the pool. Must be > 0. |
| `auto_migrate` | boolean | `true` | Run auto-migrations on startup and reload. |
| `allow_destructive` | boolean | `false` | **DESTRUCTIVE**: Drop columns missing from YAML config. See warning below. |
| `acquire_timeout` | integer (seconds) | `5` | How long to wait for a connection from the pool. |

**Validation**:
```
databases.<name>.url must not be empty
databases.<name>.max_connections must be > 0
databases.<name>.min_connections (X) must be <= max_connections (Y)
```

### 7.2 Auto-Migration

When `auto_migrate` is `true` (default), Main Serve runs SQL migrations on startup and reload for all tables referencing that database. The migration system:

1. Compares the YAML-defined column schema against the actual database schema.
2. **Adds** columns that exist in YAML but not in the database.
3. **Adds** indexes for columns with `indexed: true`.
4. **Adds** foreign key constraints defined in `foreign_keys`.
5. **Creates** join tables for `content_references` if needed.

**Safe mode (default)**: Columns present in the database but missing from the YAML config are logged as warnings and left untouched. No data is lost.

**Destructive mode**: **WARNING: THIS IS DESTRUCTIVE AND CANNOT BE UNDONE**. When `allow_destructive: true`, columns missing from the YAML config are dropped from the database via `ALTER TABLE ... DROP COLUMN`. Any data in those columns is permanently deleted.

Migration is driven by the sqlx migration system. The `url` field uses the standard connection string format for each driver (e.g., `postgres://user:pass@host/db`, `sqlite:///path/to/db.sqlite`).

## 8. Table Schemas

Table schemas define the structure of database tables. They are used for auto-migration and CRUD query generation.

### 8.1 Structure

Each table entry:

```yaml
- name: "users"
  database: "main"        # references a key in `databases`
  columns: [...]          # column definitions
  foreign_keys: [...]     # optional foreign key constraints
```

**Validation**:
```
tables[0] (users): references database 'main' which is not defined in databases
tables[0] (users): must have at least one column
tables[0] (users): must have at least one primary key column
tables[0] (users): has duplicate column name 'email'
```

### 8.2 Column Types

| Type | SQL DDL | Notes |
|------|---------|-------|
| `integer` | `INTEGER` | 32-bit signed |
| `bigint` | `BIGINT` | 64-bit signed |
| `smallint` | `SMALLINT` | 16-bit signed |
| `serial` | `SERIAL` | Auto-incrementing 32-bit (Postgres); maps to `INTEGER PRIMARY KEY AUTOINCREMENT` in SQLite |
| `bigserial` | `BIGSERIAL` | Auto-incrementing 64-bit (Postgres); maps to `BIGINT PRIMARY KEY AUTOINCREMENT` in SQLite |
| `text` | `TEXT` | Variable-length |
| `varchar` | `VARCHAR(n)` | Variable-length with optional length limit |
| `char` | `CHAR(n)` | Fixed-length |
| `boolean` | `BOOLEAN` | True/false |
| `float` | `FLOAT` / `REAL` | 32-bit IEEE 754 |
| `double` | `DOUBLE PRECISION` | 64-bit IEEE 754 |
| `decimal` | `DECIMAL(p, s)` | Exact decimal (Postgres) |
| `date` | `DATE` | Calendar date |
| `timestamp` | `TIMESTAMP` | Without timezone |
| `timestamptz` | `TIMESTAMP WITH TIME ZONE` | With timezone |
| `uuid` | `UUID` | 128-bit UUID (Postgres/SQLite with extension) |
| `json` | `JSON` | JSON data (Postgres) / text (SQLite, stored as TEXT) |
| `jsonb` | `JSONB` | Binary JSON (Postgres) / JSON (SQLite via JSON1 extension) |
| `blob` | `BLOB` | Binary data |
| `bytea` | `BYTEA` | Binary data (Postgres alias for BLOB) |

### 8.3 Column Constraints

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `primary_key` | boolean | `false` | Marks column as primary key. Primary key columns are always `NOT NULL` in generated DDL, regardless of `nullable`. |
| `nullable` | boolean | `true` | Whether NULL is allowed. |
| `default` | string | `null` | Default SQL expression (e.g., `"now()"`, `"'active'"`, `"0"`). |
| `unique` | boolean | `false` | Creates a `UNIQUE` constraint. |
| `indexed` | boolean | `false` | Creates an index on this column. |

### 8.4 Foreign Keys

```yaml
foreign_keys:
  - column: "org_id"
    references_table: "organizations"
    references_column: "id"
    on_delete: "cascade"     # cascade, set_null, restrict, no_action (default)
    on_update: "cascade"     # same options as on_delete
```

**Validation**:
```
tables[0] (users): foreign_keys references column 'org_id' which does not exist
tables[0] (users): foreign_keys references table 'organizations' which is not defined
```

The referenced table must exist in the same database connection.

### 8.5 Table+Database Pair Uniqueness

Each `table.name + table.database` combination must be unique across the entire config. Duplicate definitions are rejected:

```
Duplicate table definition for 'users.main'
```

## 9. JSONB Schema Validation

Main Serve enforces JSON Schema (draft 2020-12) validation on `jsonb` and `json` columns at runtime.

### 9.1 Three Mutually Exclusive Mechanisms

At most one of these may be set on a single column:

| Field | Type | Description |
|-------|------|-------------|
| `validation_schema` | string (file path) | Path to external JSON Schema file. Resolved relative to the config file's parent directory. |
| `validation` | object (inline JSON Schema) | Inline JSON Schema document. |
| `validation_schema_ref` | string (dotted reference) | Reference to a named schema in `global_schemas`. Accepts both `"name"` and `"global_schemas.name"` formats. |

**Validation**:
```
tables[0] (users): column 'metadata' has schema validation but type 'varchar' is not jsonb or json
tables[0] (users): column 'metadata' has multiple schema validation fields set (validation_schema, validation); at most one is allowed
```

### 9.2 Schema Compilation at Startup

The `SchemaRegistry` builds validators once at config load time:

1. Compiles all `global_schemas` entries.
2. For each table's JSONB/JSON columns, resolves the configured schema (external file -> inline -> global ref).
3. Stores per-column validators keyed by `(table_name, column_name)`.
4. All compiled schemas are shared across requests via `Arc`.

### 9.3 Runtime Validation Behavior Per HTTP Method

| Method | Behavior |
|--------|----------|
| **POST** (create) | Validates all JSONB fields present in the request body. |
| **PUT/PATCH** (update) | Validates only the JSONB fields present in the request body. Fields not present are skipped. |
| **GET/DELETE** | Validation does not run. |
| Any method | Only values explicitly provided in the body are validated. SQL defaults (e.g., `DEFAULT now()`) are not validated. |

### 9.4 Error Format

On validation failure, the endpoint returns **HTTP 400 Bad Request** with the standard error response body:

```json
{
  "error": {
    "code": "bad_request",
    "message": "JSONB validation failed: metadata: must contain required property 'title'",
    "details": ["metadata: must NOT be null"]
  }
}
```

The `details` array contains per-column violation messages when available. The standard error response format applies to all `AppError` variants throughout the application.

### 9.5 Non-JSONB Body Keys

Body keys that are not JSONB columns are silently skipped by the validator. They do not cause validation errors but are handled by the column's type system independently.

## 9.6 Global Schemas

Named JSON Schema documents defined at the top level under `global_schemas:`. They are compiled once at config load time and referenced by columns via `validation_schema_ref`.

```yaml
global_schemas:
  blog_post:
    type: "object"
    required: ["title", "body"]
    properties:
      title: { type: "string" }
      body: { type: "string" }
      tags:
        type: "array"
        items: { type: "string" }
    additionalProperties: false
```

Each key is a unique schema name. Columns reference it with `validation_schema_ref: "global_schemas.blog_post"`. The same compiled schema can be shared across multiple columns/tables, reducing compilation overhead.

## 10. Authentication

Main Serve supports four authentication providers, defined globally under `auth:`. Endpoints reference providers by name (`"jwt"`, `"api_key"`, `"basic"`, `"oauth2"`) or `"none"`.

### 10.1 JWT Authentication

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `secret` | string | required | HMAC secret or path to public key for RSA/EC algorithms. Use `${ENV_VAR}`. |
| `algorithm` | enum | `"HS256"` | `HS256`, `HS384`, `HS512`, `RS256`, `RS384`, `RS512`, `ES256`, `ES384`. |
| `issuer` | string | `"main-serve"` | Expected `iss` claim. Empty = no validation. |
| `audience` | string | `""` | Expected `aud` claim. Empty = no validation. |
| `expiry` | integer (seconds) | `3600` (1 hour) | Token lifetime. |
| `role_claim` | string | `"role"` | JWT claim containing the user's role. |
| `revocation` | object | `null` | Token revocation configuration. |

**Revocation** (optional):

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `store` | enum | `"in_memory"` | `in_memory` or `database`. |
| `db_table` | string | `"token_blacklist"` | Database table for revocation (only when `store: database`). |
| `cleanup_interval_secs` | integer | `3600` | Cleanup interval for expired revocation entries (only when `store: database`). |

**JWT revocation behavior**: When configured, JWTs with a `jti` (JWT ID) claim are checked against the revocation store before being accepted. Tokens present in the blacklist are rejected regardless of expiry.

**Token introspection**: On every authenticated request, Main Serve verifies the JWT signature using the configured algorithm and key material, validates `iss`/`aud` if configured, checks expiry, and optionally checks revocation status. The user's role is extracted from the `role_claim`.

**Validation**:
```
auth.jwt.secret must not be empty
auth.jwt.revocation.db_table must not be empty when store is database
auth.jwt.revocation.cleanup_interval_secs must be > 0
```

### 10.2 API Key Authentication

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `location` | enum | `"header"` | Where to look: `header` or `query`. |
| `name` | string | `"X-API-Key"` | Header name or query parameter name. |
| `keys` | list | required | Valid API keys with optional roles. |

Each key entry:

```yaml
- key: "${API_KEY_1}"
  role: "admin"     # optional
```

**Validation**:
```
auth.api_key.keys must contain at least one key
```

### 10.3 HTTP Basic Authentication

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `realm` | string | `"main-serve"` | Realm in `WWW-Authenticate` header. |
| `users` | list | required | Valid users with argon2-hashed passwords. |

Each user:

```yaml
- username: "admin"
  password_hash: "${ADMIN_PASSWORD_HASH}"   # argon2 hash
  role: "admin"                             # optional
```

### 10.4 OAuth2

Main Serve supports two OAuth2 modes:

**Mode 1 -- Token Introspection** (`userinfo_url` only):

Endpoints use `auth: "oauth2"`. Main Serve validates Bearer tokens by calling the configured `userinfo_url`, which returns JSON with `sub` (subject/user ID) and optional `role`.

**Mode 2 -- Authorization Code Flow with PKCE** (`authorization_url` set):

Main Serve acts as the OAuth2 client. When an endpoint with `auth: "oauth2"` receives an unauthenticated request:

1. Main Serve redirects the user to the IdP's authorization endpoint with a PKCE challenge and state parameter.
2. The user logs in at the IdP and authorizes the app.
3. The IdP redirects back to `redirect_url` with an authorization code.
4. Main Serve exchanges the code at `token_url` for tokens.
5. Main Serve fetches user info from `userinfo_url`.
6. Main Serve mints its own JWT (requires `auth.jwt` to be configured) and sets it as an HttpOnly cookie (`cookie_name`).
7. The user is redirected to `success_url`.

**Code flow endpoints are auto-registered at**:
- `GET /_main-serve/oauth2/authorize` -- redirects to IdP
- `GET /_main-serve/oauth2/callback` -- handles IdP callback

**Required fields for code flow**:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `authorization_url` | string | `""` | IdP authorization endpoint. Setting this enables code flow. |
| `token_url` | string | required (with code flow) | Token exchange endpoint. |
| `userinfo_url` | string | required | OIDC userinfo endpoint. |
| `client_id` | string | required (with code flow) | OAuth2 client ID. |
| `client_secret` | string | required | OAuth2 client secret. |
| `redirect_url` | string | required (with code flow) | Callback URL registered with the IdP. |

Optional:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `scopes` | list | `["openid"]` | Requested OAuth2 scopes. |
| `success_url` | string | `"/"` | Redirect URL after successful login. |
| `cookie_name` | string | `"main_serve_token"` | HttpOnly cookie name for the minted JWT. |
| `state_ttl` | integer | `300` (5 minutes) | Max lifetime for pending authorization states. |
| `max_pending_states` | integer | `1000` | Max concurrent pending authorization flows. |

**Role mapping** (optional):

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default_role` | string | `"user"` | Role when no IdP role maps. |
| `role_claim` | string | `"groups"` | Claim in userinfo response containing IdP roles. |
| `role_map` | map | required (with role_mapping) | IdP role -> Main Serve role. |
| `match_mode` | enum | `"exact"` | Matching mode: `exact` or `contains`. |

**Validation**:
```
auth.jwt must be configured when OAuth2 code flow is enabled
auth.oauth2.token_url must not be empty when authorization_url is set
auth.oauth2.client_id must not be empty when authorization_url is set
auth.oauth2.redirect_url must not be empty when authorization_url is set
auth.oauth2.role_mapping.role_map must not be empty when role_mapping is configured
```

### 10.5 User Registration

```yaml
auth:
  register:
    enabled: true
    table: "users"
    database: "main"
    default_role: "user"
    password_hash: "argon2id"
```

Enables self-service signup. Passwords are hashed with the configured algorithm (only `argon2id` is supported). New users receive `default_role`.

**Validation**:
```
auth.register.table must not be empty when registration is enabled
auth.register.database must not be empty when registration is enabled
auth.register.default_role must not be empty
```

### 10.6 Role Inheritance

```yaml
role_hierarchy:
  admin:
    - editor
  editor:
    - author
```

A role inherits all permissions of its listed parent roles transitively. `admin` inherits from `editor`, which inherits from `author`. Self-references and cycles are detected and rejected:

```
role_hierarchy.admin: role cannot list itself as a parent
role_hierarchy: cycle detected: admin -> editor -> admin
```

## 11. Storage Backends

Named storage backends are defined under `stores:`. Each key is referenced by endpoints.

### 11.1 Backend Types

| Backend | Additional Config | Notes |
|---------|-------------------|-------|
| `native` | `root` (required) | Local disk storage. Main Serve should run as root or have write access to `root`. |
| `s3` | `s3.region`, `s3.bucket`, `s3.access_key`, `s3.secret_key` (all required). Optional: `s3.endpoint`, `s3.force_path_style`. | Amazon S3 or S3-compatible services. Requires the `s3` feature flag to be enabled. |
| `azure` | `azure.account_name`, `azure.account_key`, `azure.container` (all required) | Azure Blob Storage. |
| `gcs` | `gcs.project_id`, `gcs.credentials`, `gcs.bucket` (all required) | Google Cloud Storage. |
| `memory` | none | In-memory storage for testing only. |

### 11.1.1 S3 Path Style

The `s3.force_path_style` option controls how the S3 client constructs URLs:

- **Virtual-hosted style** (default, `force_path_style: false`): `bucket.s3.amazonaws.com/key`
- **Path style** (`force_path_style: true`): `s3.amazonaws.com/bucket/key`

Path style is required for S3-compatible services like MinIO, LocalStack, and Ceph.

### 11.2 Mutual Exclusivity

At most one backend-specific config section may be present per store. Validation enforces:

```
stores.<name>: conflicting backend config sections: ["root", "s3"] (at most one allowed)
stores.<name>: root must not be set for memory backend
```

### 11.3 Cross-Reference Validation

All `storage:` references in endpoints are validated at config load time. If an endpoint references a store that does not exist in `stores:`, the error is:

```
stores.<name> referenced by endpoint <path> does not exist in stores
```

### 11.4 Store Root: Native vs Cloud

For endpoints that reference a store (e.g., `static_files`, `media`, `file_store`, `spa_host`), the `root` field in the store config has different semantics depending on the backend:

- **Native**: `root` is the actual filesystem directory that becomes the serving root. Files are read from and written to this directory.
- **S3 / Azure / GCS**: `root` is conceptual. For S3, it represents the bucket name. Files are stored in the bucket/container itself, not in a subdirectory. The `root` field is required for structural consistency but its value is not used as a path prefix for cloud backends.

This distinction matters when configuring `create_subdirectory` patterns in uploads: on native stores, files land in `{root}/{subdirectory}/...`; on cloud stores, files land in `{bucket}/{subdirectory}/...`.

## 12. Endpoints

### 12.1 Path Patterns

| Pattern | Match | Example |
|---------|-------|---------|
| `/api/users` | Exact path | Matches `/api/users` only. |
| `/api/users/{id}` | Single segment | Matches `/api/users/42`. `{id}` is captured as a path parameter. |
| `/assets/*` | Trailing wildcard | Matches `/assets/foo.jpg`, `/assets/dir/file.png`, etc. `*` captures the remainder of the path. |

### 12.2 HTTP Methods

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `methods` | list | required | HTTP methods this endpoint responds to: `get`, `post`, `put`, `patch`, `delete`, `head`, `options`. |

### 12.3 Action Dispatch

The `action` field determines the endpoint's behavior. Only one action-specific config block is required per endpoint. Valid actions: `crud`, `proxy`, `static_files`, `media`, `file_store`, `spa_host`, `custom_response`.

### 12.4 Auth/Roles Inheritance

| Field | Default | Description |
|-------|---------|-------------|
| `auth` | `"none"` | Auth provider name. Endpoints with `auth: "none"` skip authentication entirely. |
| `roles` | `[]` (empty = any authenticated user) | Required roles. |

### 12.5 CORS/Rate Limit Inheritance

| Field | Description |
|-------|-------------|
| `cors` | Per-endpoint CORS override. Omit to inherit from global config. |
| `rate_limit` | Per-endpoint rate limit override. Omit to inherit from global config. |

## 13. Action Types

### 13.1 CRUD

Generates SQL queries against a database table with support for pagination, filtering, sorting, joins, computed fields, and request-context interpolation.

#### Required Fields

| Field | Type | Description |
|-------|------|-------------|
| `table` | string | Table name (must exist in `tables:`). |
| `database` | string | Named database (must exist in `databases:`). |

Cross-reference validation at load time:
```
endpoints[0] (/api/users): crud.table 'users' is not defined in tables
endpoints[0] (/api/users): crud.database 'main' is not defined in databases
```

#### Field Selection

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `fields` | list of strings | `["*"]` | Columns returned in GET responses. `["*"]` returns all columns. |
| `writable_fields` | list of strings | all non-PK columns | Columns writable on POST/PUT/PATCH. Omit to allow all non-PK fields. |

#### Pagination

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `pagination.enabled` | boolean | `true` | Enable pagination. |
| `pagination.default_page_size` | integer | `20` | Default page size. |
| `pagination.max_page_size` | integer | `100` | Maximum allowed page size (enforced client-side). |

Client queries use `?page=N&per_page=N` parameters.

#### Filtering

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `filtering.enabled` | boolean | `true` | Enable filtering. |
| `filtering.allowed_fields` | list of strings | `["*"]` | Columns that can be filtered on. `["*"]` allows all. |

Filtering syntax: `?field=value` for exact match, `?field>value` for comparison. For JSONB columns, dot-notation and LHS brackets are supported:

```
?metadata.role=admin
?metadata[role]=admin
```

#### Sorting

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `sorting.enabled` | boolean | `true` | Enable sorting. |
| `sorting.default_field` | string | primary key | Default sort column. |
| `sorting.default_order` | enum | `"asc"` | Default sort order: `asc` or `desc`. |
| `sorting.allowed_fields` | list of strings | `["*"]` | Columns that can be sorted on. |

Client queries use `?sort=field&order=asc|desc` parameters. JSONB columns support dot-notation and LHS brackets.

#### WHERE Clauses

| Field | Type | Description |
|-------|------|-------------|
| `where_clause` | string | Applied to ALL queries on this endpoint. Static or dynamic. |
| `update_where_clause` | string | Additional WHERE clause for UPDATE only. Combined with `where_clause` via AND. |
| `delete_where_clause` | string | Additional WHERE clause for DELETE only. Combined with `where_clause` via AND. |

**Dynamic interpolation** (`${request.user.id}`, `${request.user.role}`, `${request.headers.NAME}`):

```yaml
crud:
  where_clause: "deleted_at IS NULL"
  update_where_clause: "author_id = ${request.user.id}"
  delete_where_clause: "author_id = ${request.user.id}"
```

**SQL safety**: WHERE clauses are validated for unsafe SQL characters (`;`, `--`, `/*`). Attempts to inject SQL are rejected at config load time:

```
endpoints[0] (/api/users): crud.where_clause contains unsafe SQL characters (;, --, /*)
```

#### Insert Owner

| Field | Type | Description |
|-------|------|-------------|
| `insert_owner` | string | Column name to auto-populate with `${request.user.id}` on INSERT. If the request body already provides this value, that value takes precedence. |

#### Joins

```yaml
joins:
  - table: "organizations"
    "on": "users.org_id = organizations.id"   # MUST be quoted (YAML 1.2 boolean)
    join_type: "left"   # inner, left, right
    fields:
      - "organizations.name as org_name"
```

The `on` field is an SQL expression and must be quoted in YAML to avoid being parsed as a boolean. The `join_type` determines the JOIN clause type.

#### Computed Fields

```yaml
computed_fields:
  - name: "full_name"
    expression: "first_name || ' ' || last_name"
```

These are added to SELECT statements as virtual columns. The `expression` is validated for unsafe SQL characters at config load time.

### 13.2 Proxy

Forwards requests to an upstream HTTP service with path rewriting, custom headers, and timeouts.

#### Required Fields

| Field | Type | Description |
|-------|------|-------------|
| `upstream` | string (URL) | Base URL for forwarding. |

**Validation**:
```
endpoints[0] (/api/external/*): proxy.upstream must not be empty
```

#### Path Rewrite

```yaml
proxy:
  path_rewrite:
    strip_prefix: "/api/external"   # stripped from incoming path
    add_prefix: "/v2"               # prepended after stripping
```

#### Headers

Map of header name to value. Values support `${ENV_VAR}` interpolation:

```yaml
headers:
  X-Forwarded-For: "client"
  Authorization: "Bearer ${UPSTREAM_TOKEN}"
```

#### Timeouts

| Field | Type | Default (seconds) | Description |
|-------|------|-------------------|-------------|
| `timeouts.connect` | integer | `5` | TCP connect timeout. |
| `timeouts.read` | integer | `30` | Response read timeout. |
| `timeouts.total` | integer | `60` | Total request timeout. |

#### Response Size Limit

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `max_response_size` | integer (bytes) | `268435456` (256 MiB) | Max upstream response body size. `0` disables the limit. |

Exceeding this limit is rejected to prevent memory exhaustion.

### 13.3 Static Files

Serves files directly from a storage backend with no database layer.

#### Required Fields

| Field | Type | Description |
|-------|------|-------------|
| `storage` | string | Named store (must exist in `stores:`). |

#### Serving Configuration

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `index` | string | `"index.html"` | Index file for directory requests. |
| `directory_listing` | boolean | `false` | Show directory listings when no index file found. |
| `cache_max_age` | integer (seconds) | `3600` | Default Cache-Control max-age. `0` disables caching. |
| `etag` | boolean | `true` | Generate ETag headers for cache validation. |
| `range_requests` | boolean | `true` | Support HTTP range requests (206 Partial Content). |
| `head_support` | boolean | `true` | Support HTTP HEAD method. |

#### Per-Extension Cache Rules

```yaml
cache_rules:
  - extensions: [".html"]
    cache_control: "no-cache"
  - extensions: [".jpg", ".png", ".gif", ".webp", ".svg"]
    cache_control: "public, max-age=31536000, immutable"
```

These override `cache_max_age` for matching extensions.

#### Upload

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `upload.enabled` | boolean | `false` | Enable file uploads. |
| `upload.max_size` | integer (bytes) | `10485760` (10 MiB) | Max upload size. |
| `upload.allowed_extensions` | list | `[]` (all) | Allowed file extensions (with dots). |
| `upload.create_subdirectory` | string | null | Subdirectory pattern: `{user_id}`, `{year}`, `{month}`, `{day}`, `{uuid}`. |
| `upload.mime_detection` | enum | `"magic"` | MIME detection: `extension` or `magic`. |

#### Streaming

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `streaming.enabled` | boolean | `false` | Enable chunked streaming for large files. |
| `streaming.buffer_size` | integer (bytes) | `65536` (64 KiB) | Chunk size. |
| `streaming.threshold` | integer (bytes) | `1048576` (1 MiB) | Files larger than this use streaming; smaller files are read entirely into memory. |
| `streaming.include_content_length` | boolean | `true` | Include Content-Length header in streaming responses. |

#### Image Resize

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `image_resize.enabled` | boolean | `false` | Enable on-demand image resizing. |
| `image_resize.max_dimension` | integer (pixels) | `4096` | Max dimension for auto-resize. |
| `image_resize.supported_formats` | list | `["jpg", "jpeg", "png", "webp"]` | Supported output formats. |
| `image_resize.default_fit` | enum | `"scale_down"` | Fit mode: `scale_down`, `cover`, `contain`. |
| `image_resize.cache_dir` | string | null | Cache directory for resized images. Full path: `{store_root}/{cache_dir}/{media_id}/{style_name}.{format}`. For cloud stores, resized images are stored in the cloud backend. |

### 13.4 Media

A full media library with upload, trash, sharing, content references, and image resizing. Requires both a storage backend and a database table.

#### Required Fields

| Field | Type | Description |
|-------|------|-------------|
| `storage` | string | Named store. |
| `table` | string | Database table for metadata. |
| `database` | string | Named database. |

All cross-reference validation (stores, tables, databases) occurs at config-load time.

#### Column Presets

| Preset | Expands To |
|--------|-----------|
| `"auto"` | `original_name` (text), `mime_type` (text), `size` (bigint), `uploader_id` (uuid), `created_at` (timestamptz), `updated_at` (timestamptz) |
| `"tags"` | `tags` (jsonb, default: null) -- API-level treated as string array |
| `"description"` | `description` (text, default: null) |
| `"alt_text"` | `alt_text` (text, default: null) |
| `"content_type"` | `content_type` (varchar with check constraint: `image`, `video`, `audio`, `document`, `other`) |

Default: `["auto"]`.

#### Custom Metadata Columns

```yaml
metadata_columns:
  - name: "license"
    type: "text"
    nullable: true
    default: "''"
```

Same structure as `tables[].columns[]` but **without** `primary_key`, `unique`, `indexed`, `check`, or `foreign_key` constraints (those are silently ignored).

#### Upload

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `upload.max_size` | integer (bytes) | `104857600` (100 MiB) | Max upload size. |
| `upload.allowed_extensions` | list | `[]` (all) | Allowed file extensions. |
| `upload.allowed_mime_types` | list | `[]` (all) | Allowed MIME types. |
| `upload.mime_detection` | enum | `"magic"` | `extension` or `magic`. |
| `upload.bulk_supported` | boolean | `true` | Support multipart/mixed bulk uploads. |
| `upload.create_subdirectory` | string | null | Pattern: `{user_id}`, `{year}`, `{month}`, `{day}`, `{uuid}`. |
| `upload.versioning` | object | disabled | File versioning config. |
| `upload.versioning.max_versions` | integer | `10` | Max versions per file. |
| `upload.preview_generation` | object | disabled | Preview generation for non-image files. |
| `upload.preview_generation.enabled` | boolean | `false` | Enable preview generation. |
| `upload.preview_generation.previews` | list | empty | Named preview styles. Each entry has `name`, `max_width`, `max_height`, `format`. |

#### Move

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `move.enabled` | boolean | `true` (when section present) | Enable move operations. |
| `move.admin_override` | boolean | `true` | Admin users can move files anywhere. Non-admins move only within their own directory tree. |
| `move.auto_create_destination` | boolean | `false` | Auto-create destination directories. |

Omit the section to disable move operations.

#### Rename

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `rename.enabled` | boolean | `true` (when section present) | Enable rename operations. |
| `rename.admin_override` | boolean | `true` | Admin users can rename any file. Non-admins rename only their own files. |

Omit the section to disable rename operations.

#### Delete

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `delete.enabled` | boolean | `true` (when section present) | Enable delete operations. |
| `delete.admin_override` | boolean | `true` | Admin users can delete any file. Non-admins delete only their own files. |

Omit the section to disable delete operations.

#### User Scoping

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `user_scope.enabled` | boolean | `true` (when section present) | Enable user-level scoping. |
| `user_scope.mode` | enum | `"shared"` | `user` (own uploads only), `shared` (all files visible, own files managed), `open` (no restrictions, role-based access only). |
| `user_scope.admin_roles` | list | `[]` | Roles that bypass user scoping entirely. |
| `user_scope.allow_cross_user_browse` | boolean | `false` | Allow browsing other users' public directories. |

Omit the section to disable user scoping.

#### Content References

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `content_references.enabled` | boolean | `true` (when section present) | Enable content entity integration. |
| `content_references.table` | string | `"media_entity_refs"` | Association table name (auto-created at startup). |
| `content_references.media_id_column` | string | `"media_id"` | FK column to media table. |
| `content_references.entity_id_column` | string | `"entity_id"` | Column referencing any content entity. |
| `content_references.content_type_column` | string | `"content_type"` | Column for content type (e.g., "post", "product"). |
| `content_references.order_column` | string | `"attachment_order"` | Sort order column. |
| `content_references.allowed_content_types` | list or null | null | Allowed content types. null = all. |
| `content_references.on_delete` | enum | `"detach"` | Behavior when media is deleted while attached: `detach` (remove association), `cascade` (delete association + media + storage file), `error` (HTTP 409 Conflict). |

The association table is auto-created with: `id` (bigserial PK), `media_id` (bigint FK), `entity_id` (bigint), `content_type` (varchar), `attachment_order` (integer, default 0). Unique constraint on `(media_id, entity_id, content_type)`.

#### Trash

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `trash.enabled` | boolean | `false` | Enable trash. Deleted files are moved, not permanently removed. |
| `trash.retention_days` | integer | `14` | Days to retain trashed files. |
| `trash.prefix` | string | `".trash"` | Store prefix for trashed files. Files moved to `{store_root}/{prefix}/{user_scope_path}/{original_path}`. |
| `trash.management_endpoints` | boolean | `true` | Expose trash management endpoints. |
| `trash.admin_roles` | list | `["admin"]` | Roles allowed to permanently delete and empty trash. |

Auto-registered endpoints when `management_endpoints: true`:
- `GET /_main-serve/media/trash` -- list trashed files
- `POST /_main-serve/media/trash/{id}/restore` -- restore a file
- `DELETE /_main-serve/media/trash/{id}` -- permanently delete
- `DELETE /_main-serve/media/trash` -- empty trash

Omit or set `enabled: false` to disable trash.

#### Sharing

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `sharing.enabled` | boolean | `false` | Enable time-limited public download links. |
| `sharing.signing_secret` | string | required (with sharing) | Secret for signing/verifying share links (use `${ENV_VAR}`). |
| `sharing.default_ttl` | integer (seconds) | `86400` (24h) | Default share link TTL. |
| `sharing.max_ttl` | integer (seconds) | `604800` (7d) | Max share link TTL. |
| `sharing.prefix` | string | `"shared"` | Store prefix for publicly accessible files. Sharing creates a physical copy at `{store_root}/{prefix}/{token}/{path}`. |
| `sharing.allow_any_user` | boolean | `true` | Any authenticated user can create share links. |
| `sharing.required_link_role` | string or null | null | Role required to create share links (only used when `allow_any_user: false`). |

Shared files are independent copies -- if the original is trashed or moved, the shared copy remains intact.

#### Pagination, Sorting, Filtering (Metadata Listing)

Same structure as CRUD pagination/filtering/sorting. Defaults:
- Pagination: `enabled: true`, `default_page_size: 20`, `max_page_size: 100`
- Sorting: `enabled: true`, `default_field: "created_at"`, `default_order: "desc"`
- Filtering: `enabled: true`, `allowed_fields` lists metadata columns

#### Faceted Search

```yaml
facets:
  - name: "type"
    field: "content_type"
    facet_type: "term"
  - name: "date"
    field: "created_at"
    facet_type: "date_range"
  - name: "size"
    field: "size"
    facet_type: "numeric"
```

#### Image Resize

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `image_resize.enabled` | boolean | `false` | Enable on-demand resizing. |
| `image_resize.max_dimension` | integer (pixels) | `4096` | Max dimension. |
| `image_resize.supported_formats` | list | `["jpg", "jpeg", "png", "webp"]` | Supported formats. |
| `image_resize.default_fit` | enum | `"scale_down"` | Fit mode. |
| `image_resize.cache_dir` | string | null | Cache directory for resized images. Full path: `{store_root}/{cache_dir}/{media_id}/{style_name}.{format}`. For cloud stores (S3/Azure/GCS), resized images are stored in the cloud backend (same store), not the local filesystem. |
| `image_resize.styles` | list | empty | Named image styles (name, max_width, max_height, resize_fit, format, quality). |
| `image_resize.generate_on_upload` | boolean | `false` | Generate resized styles automatically on upload. |

### 13.5 SPA Host

Serves a single-page application with automatic fallback routing.

#### Required Fields

| Field | Type | Description |
|-------|------|-------------|
| `storage` | string | Named store. |

#### Configuration

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `index` | string | `"index.html"` | SPA entry point file. |
| `cache_max_age` | integer (seconds) | `3600` | Default Cache-Control max-age. |
| `cache_rules` | list | empty | Per-extension Cache-Control rules. |
| `etag` | boolean | `true` | Generate ETag headers. |
| `head_support` | boolean | `true` | Support HEAD requests. |
| `fallback_status` | integer | `200` | HTTP status for fallback responses (non-existent paths). |

**Fallback routing**: When a non-existent path is requested, Main Serve returns the `index` file (typically `index.html`) with `fallback_status` (default 200). The SPA's client-side router then handles the path.

**No directory browsing**: SPA Host never exposes directory listings. All requests resolve to the SPA entry point.

### 13.6 File Store

Database-backed file catalog with CRUD operations and ownership tracking. Does not serve files directly -- configure a separate `static_files` endpoint pointing at the same store for that.

#### Required Fields

| Field | Type | Description |
|-------|------|-------------|
| `storage` | string | Named store. |
| `table` | string | Database table acting as the file registry. |
| `database` | string | Named database. |

#### Metadata Columns

```yaml
metadata_columns:
  - name: "original_name"
    type: "text"
  - name: "mime_type"
    type: "text"
  - name: "size"
    type: "bigint"
  - name: "uploader_id"
    type: "uuid"
  - name: "tags"
    type: "jsonb"
    nullable: true
    default: "null"
  - name: "description"
    type: "text"
    nullable: true
    default: "null"
  - name: "alt_text"
    type: "text"
    nullable: true
    default: "null"
```

#### Field Permissions

```yaml
field_permissions:
  original_name:
    read: ["admin", "editor"]
    write: ["admin"]
  mime_type:
    read: ["admin", "editor", "author"]
    write: ["admin"]
  tags:
    read: ["admin", "editor"]
    write: ["admin", "editor"]
  description:
    read: ["admin", "editor"]
    write: ["admin", "editor", "author"]
  alt_text:
    read: ["admin", "editor"]
    write: ["admin", "editor", "author"]
```

For auto-generated columns NOT listed (`id`, `created_at`, `updated_at`, `uploader_id`):
- `read`: `["*"]` (all roles)
- `write`: `[]` (no roles -- write is implicit on upload)

#### Ownership

```yaml
ownership:
  owner_column: "uploader_id"    # column storing the owner ID
  admin_override: true           # admin users can access any row
```

#### Trash

```yaml
trash:
  enabled: true
  retention_days: 30
```

Default retention for file store trash is 30 days (vs. 14 days for media trash).

#### Pagination, Sorting, Filtering

Same structure as CRUD and media listing configs.

### 13.7 Custom Response

Returns a static response body with configurable status code, content type, and headers.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `status` | integer | `200` | HTTP status code (100-599). |
| `content_type` | string | `"application/json"` | Response Content-Type. |
| `body` | string | required | Response body (inline string). |
| `headers` | map | `null` | Additional response headers. |

**Validation**:
```
endpoints[0] (/api/info): custom_response.status must be a valid HTTP status code (100-599)
```

## 14. Error Handling

All `AppError` variants produce a consistent JSON response with the standard error body:

```json
{
  "error": {
    "code": "<error_code>",
    "message": "<human-readable message>",
    "details": ["<additional details>"]
  }
}
```

The `details` field is omitted when no additional information is available. Each variant has a machine-readable `code`:

| Variant | Error Code |
|---------|------------|
| `AppError::Config(_)` | `config_error` |
| `AppError::Validation(_)` | `validation_error` |
| `AppError::Database(_)` | `database_error` |
| `AppError::Auth(_)` / `AuthChallenge(_, _)` | `auth_error` |
| `AppError::Forbidden(_)` | `forbidden` |
| `AppError::NotFound(_)` | `not_found` |
| `AppError::MethodNotAllowed(_)` | `method_not_allowed` |
| `AppError::RateLimited` | `rate_limited` |
| `AppError::BadRequest(_)` | `bad_request` |
| `AppError::PayloadTooLarge(_)` | `payload_too_large` |
| `AppError::ServiceUnavailable(_)` | `service_unavailable` |
| `AppError::FileOperation(_)` | `file_operation` |
| `AppError::Conflict { .. }` | `conflict` |
| `AppError::Internal(_)` | `internal_error` |
| `AppError::Io(_)` | `io_error` |
| `AppError::Body(_)` | `request_body_error` |
| `AppError::ParseError(_)` | `json_parse_error` |
| `AppError::RequestedRangeNotSatisfiable(_)` | `range_not_satisfiable` |

**Special headers:**
- `AuthChallenge(_, _)` adds a `WWW-Authenticate` header to the response.
- `RateLimited(Some(secs))` adds a `Retry-After` header with the number of seconds until the rate limit window resets.
- `MethodNotAllowed { allowed, .. }` adds an `Allow` header with the comma-separated list of allowed methods when non-empty.
- `RequestedRangeNotSatisfiable { content_range, .. }` adds a `Content-Range` header when specified.

**Database error sanitization:** `AppError::Database` returns the hardcoded message `"A database error occurred"` to avoid leaking SQL/connection details.

**Internal error sanitization:** `AppError::Config`, `AppError::Validation`, `AppError::FileOperation`, `AppError::Internal`, and `AppError::Io` all return the hardcoded message `"An internal error occurred"` to clients. The actual error message is always logged via `tracing::error!` for debugging but never exposed in the HTTP response. This prevents leaking internal paths, connection strings, or implementation details.

### 14.1 Config Load Errors

| Error | Cause |
|-------|-------|
| `Failed to resolve config path <path>: <err>` | Config file path does not exist or cannot be canonicalized. |
| `Failed to read config file <path>: <err>` | Permission denied or I/O error. |
| `Failed to parse YAML in <path>: <err>` | Invalid YAML syntax. |
| `Environment variable interpolation failed: ...` | One or more `${VAR}` without default, where `VAR` is unset. |
| `Circular $include detected: <path> was already included` | Circular `$include` chain. |
| `$include value must be a string, got <type>` | `$include` target is not a string. |
| `$include in a mapping must be the only key` | Mixing `$include` with other keys in a mapping. |
| `$include in a sequence item must be the only key` | Mixing `$include` with other keys in a sequence item. |
| `$include pattern '<pattern>' matched no files` | Glob pattern with no matches. |
| `Configuration validation failed:\n  - <errors>` | All semantic validation failures combined. |

### 14.2 HTTP Errors

| Status | Trigger | Description |
|--------|---------|-------------|
| **400** | Bad request | Malformed request body or parameters. Includes JSONB validation failures, parse errors, and missing required fields. |
| **401** | Authentication failure | Invalid or missing credentials. `AuthChallenge` variants include a `WWW-Authenticate` header. |
| **403** | Role mismatch | Authenticated user lacks required role. |
| **404** | Not found | Resource or endpoint not found. |
| **405** | Method not allowed | HTTP method not permitted for the resource. Response includes an `Allow` header listing permitted methods. |
| **409** | Conflict | Request conflicts with current server state (e.g., deleting media attached to content entities when `on_delete: error`). Response `details` array lists the conflicting resources. |
| **413** | Payload too large | Request body or file upload exceeds configured size limit. |
| **416** | Range not satisfiable | Requested byte range exceeds file size. Response may include a `Content-Range` header. |
| **429** | Rate limited | Client exceeded rate limit threshold. Response includes a `Retry-After` header. |
| **500** | Internal error | Unexpected server error. Includes config errors, validation errors, database errors (sanitized), file operation errors, and other internal failures. |
| **503** | Service unavailable | Server temporarily unable to handle request (e.g., upstream dependency down, resource exhausted). |

## 15. Hot Reload

Main Serve supports live configuration reload without restart.

### 15.1 Trigger Endpoint

| Field | Value |
|-------|-------|
| Method | `POST` |
| Path | `/_main-serve/reload` |
| Auth | Admin token required (any configured admin role or auth provider that grants admin). |

### 15.2 Reload Process

1. The full config loading pipeline runs (read -> interpolate env vars -> resolve includes -> parse YAML -> deserialize -> validate).
2. If validation passes, the new config is atomically swapped:
   - All database connection pools are rebuilt (existing connections are drained first).
   - All storage backends are re-initialized.
   - The `SchemaRegistry` is rebuilt with new JSON Schema validators.
   - All endpoint routes are re-registered against the axum router.
3. If validation fails, the reload is aborted and the previous config remains in effect. An error response is returned with the combined validation errors.

### 15.3 Safety Guarantees

- The swap is atomic -- there is never a window where partially-loaded config is active.
- Existing in-flight requests continue to use the old configuration until they complete.
- New requests after the swap use the new configuration.
- Database pools are rebuilt gradually (new connections use the new config, old connections are drained).
- Storage backends are re-initialized without affecting in-progress file operations.

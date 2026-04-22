# Configuration Reference

This document provides detailed documentation for every configuration field in Main Serve. For a quick annotated example, see [spec.yaml](../config/spec.yaml). For a hands-on introduction, see the [Quickstart Guide](QUICKSTART.md).

---

## Table of Contents

- [Global Concepts](#global-concepts)
  - [JSON Schema Support](#json-schema-support)
  - [Environment Variable Interpolation](#environment-variable-interpolation)
  - [File Includes](#file-includes)
- [Server](#server)
  - [TLS](#tls)
- [Logging](#logging)
- [CORS](#cors)
- [Rate Limiting](#rate-limiting)
- [Databases](#databases)
- [Table Schemas](#table-schemas)
  - [Columns](#columns)
  - [Foreign Keys](#foreign-keys)
- [Authentication](#authentication)
  - [JWT](#jwt)
  - [API Key](#api-key)
  - [HTTP Basic](#http-basic)
  - [OAuth2 / OIDC](#oauth2--oidc)
- [Endpoints](#endpoints)
  - [CRUD Endpoints](#crud-endpoints)
  - [Proxy Endpoints](#proxy-endpoints)
  - [Static File Endpoints](#static-file-endpoints)
  - [Custom Response Endpoints](#custom-response-endpoints)
- [System Endpoints](#system-endpoints)

---

## Global Concepts

### JSON Schema Support

The Main Serve JSON schema for the YAML configuration file is located in this repository and you can access it [here](https://raw.githubusercontent.com/dekoding/main-serve/refs/heads/main/main-serve.json). You can use it for completions and hints in environments that support the [YAML Language Server](https://github.com/redhat-developer/yaml-language-server) by adding it to the top of a configuration file:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/dekoding/schemas/refs/heads/main/main-serve.json
server: ...
```

### Environment Variable Interpolation

Any string value in the YAML configuration supports environment variable substitution. This is the recommended way to inject secrets (database passwords, JWT secrets, API keys) without storing them in the config file.

**Syntax:**

| Pattern | Behavior |
|---|---|
| `${VAR_NAME}` | Replaced with the value of `VAR_NAME`. **Errors if the variable is not set.** |
| `${VAR_NAME:-default}` | Replaced with the value of `VAR_NAME` if set, otherwise uses `default`. |

Variable names must start with a letter or underscore, followed by letters, digits, or underscores (`[A-Za-z_][A-Za-z0-9_]*`).

**Examples:**

```yaml
databases:
  main:
    url: "${DATABASE_URL}"                    # Required - fails if not set
auth:
  jwt:
    secret: "${JWT_SECRET:-dev-secret}"       # Falls back to "dev-secret"
  api_key:
    keys:
      - key: "${API_KEY_1}"
```

Interpolation is resolved at config load time, before YAML parsing. This means you can use it in any string field, including within URLs, paths, and header values.

### Request Context Interpolation

Certain fields (like `where_clause`) support dynamic interpolation using the `${key}` syntax to inject request-specific data at runtime. This allows for object-level authorization and ownership logic.

**Supported Keys:**

| Key | Description |
|---|---|
| `${request.user.id}` | The unique identifier of the authenticated user. |
| `${request.user.role}` | The assigned role of the authenticated user. |
| `${request.headers.NAME}` | The value of the specified request header (`NAME`). |

**Examples:**

```yaml
endpoints:
  - path: "/api/posts"
    action: "crud"
    crud:
      # Only allow users to see their own posts
      where_clause: "author_id = ${request.user.id}"
```

Interpolation is resolved at query execution time. If a key cannot be resolved, the query will fail with a `400 Bad Request` error.

### File Includes

Large configurations can be split across multiple files using the `$include` directive. All paths are relative to the file containing the directive. Glob patterns (e.g., `*.yaml`, `endpoints/*.yaml`) are supported.

**In a sequence** (e.g., the `endpoints` list):

```yaml
endpoints:
  - $include: "endpoints/users.yaml"       # Single file -> one sequence item
  - $include: "endpoints/*.yaml"           # Glob -> one item per matched file
  - path: "/api/inline"                    # Inline entries still work
    action: "custom_response"
```

Each matched file should contain a single endpoint definition (without the leading `-`).

**In a mapping** (e.g., `databases`, `tables`):

```yaml
databases:
  $include: "db/databases.yaml"            # Replaces the entire mapping

tables:
  $include: "tables/*.yaml"                # Merges all matched files
```

Matched files must themselves be YAML mappings. When multiple files match a glob, their keys are merged.

**Rules:**
- Includes can be nested (an included file can itself use `$include`).
- Circular includes are detected and rejected with an error.
- Included files support `${ENV_VAR}` interpolation.

---

## Server

Controls the HTTP listener, worker threads, request limits, and TLS.

```yaml
server:
  host: "127.0.0.1"
  port: 8080
  workers: 0
  max_body_size: 10485760
  keep_alive: 75
  shutdown_timeout: 30
  tls:
    cert: "/path/to/cert.pem"
    key: "/path/to/key.pem"
```

| Field | Type | Default | Description |
|---|---|---|---|
| `host` | string | `"127.0.0.1"` | IP address to bind the HTTP listener to. Use `"0.0.0.0"` to listen on all interfaces. |
| `port` | integer (1-65535) | `8080` | Port number for the HTTP listener. |
| `workers` | integer | `0` | Number of tokio worker threads. `0` means automatic (uses the number of CPU cores). |
| `max_body_size` | integer | `10485760` (10 MiB) | Maximum request body size in bytes. Requests exceeding this limit receive a `413 Payload Too Large` response. |
| `keep_alive` | integer | `75` | HTTP keep-alive timeout in seconds. When greater than 0, idle connections are closed after this duration without a new request. Set to `0` to disable keep-alive. |
| `shutdown_timeout` | integer | `30` | Graceful shutdown timeout in seconds. After receiving SIGTERM or SIGINT, the server waits this long for in-flight requests to complete before forcefully shutting down. |
| `tls` | object | *(omitted)* | TLS configuration. Omit this section entirely to serve plain HTTP. See [TLS](#tls). |

### TLS

When the `tls` section is present, the server uses HTTPS via rustls (no OpenSSL dependency).

| Field | Type | Required | Description |
|---|---|---|---|
| `cert` | string (file path) | Yes | Path to a PEM-encoded certificate file. Can be a certificate chain. |
| `key` | string (file path) | Yes | Path to a PEM-encoded private key file. |

**Example:**

```yaml
server:
  host: "0.0.0.0"
  port: 443
  tls:
    cert: "/etc/ssl/certs/myapp.pem"
    key: "/etc/ssl/private/myapp-key.pem"
```

---

## Logging

Controls structured logging output.

```yaml
logging:
  level: "info"
  format: "pretty"
  log_request_body: false
  log_response_body: false
  max_body_log_size: 16384
```

| Field | Type | Default | Description |
|---|---|---|---|
| `level` | enum | `"info"` | Minimum log level. One of: `trace`, `debug`, `info`, `warn`, `error`. The `RUST_LOG` environment variable takes precedence if set. |
| `format` | enum | `"pretty"` | Output format. `"pretty"` for human-readable colored output, `"json"` for structured JSON logs (recommended for production). |
| `log_request_body` | boolean | `false` | When `true`, request bodies are logged at `debug` level. Bodies larger than `max_body_log_size` are truncated. Adds memory overhead since the body must be buffered. |
| `log_response_body` | boolean | `false` | When `true`, response bodies are logged at `debug` level. Bodies larger than `max_body_log_size` are truncated. Adds memory overhead since the body must be buffered. |
| `max_body_log_size` | integer | `16384` (16 KiB) | Maximum body size in bytes that will be included in log output. Bodies larger than this are truncated. Applies to both request and response body logging. |

> **Note:** Body logging is intended for development and debugging. It can expose sensitive data and adds overhead - do not enable in production.

---

## CORS

Cross-Origin Resource Sharing settings. These are the global defaults that apply to all endpoints. Individual endpoints can override these settings (see [Endpoints](#endpoints)).

```yaml
cors:
  allowed_origins:
    - "*"
  allowed_methods:
    - "GET"
    - "POST"
    - "PUT"
    - "PATCH"
    - "DELETE"
    - "OPTIONS"
    - "HEAD"
  allowed_headers:
    - "*"
  allow_credentials: false
  max_age: 86400
```

| Field | Type | Default | Description |
|---|---|---|---|
| `allowed_origins` | list of strings | `["*"]` | Origins allowed to make cross-origin requests. Use `["*"]` to allow all origins. For specific origins, list them individually: `["https://example.com", "https://app.example.com"]`. |
| `allowed_methods` | list of strings | `["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "HEAD"]` | HTTP methods allowed in cross-origin requests. Values: `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `OPTIONS`, `HEAD`. |
| `allowed_headers` | list of strings | `["*"]` | Request headers allowed in cross-origin requests. Use `["*"]` to allow all headers. Common values: `Content-Type`, `Authorization`, `X-API-Key`. |
| `allow_credentials` | boolean | `false` | Whether the browser should include credentials (cookies, authorization headers, TLS client certificates) in cross-origin requests. **Note:** When `true`, `allowed_origins` cannot be `["*"]` - you must list specific origins. |
| `max_age` | integer | `86400` (24 hours) | How long (in seconds) browsers can cache the results of a preflight (`OPTIONS`) request. |

---

## Rate Limiting

Controls request rate limiting. These are the global defaults; individual endpoints can override them.

```yaml
rate_limit:
  enabled: false
  max_requests: 100
  window_seconds: 60
  key_strategy: "ip"
  key_header: ""
  cleanup_threshold: 10000
```

| Field | Type | Default | Description |
|---|---|---|---|
| `enabled` | boolean | `false` | Whether rate limiting is active. Must be set to `true` to enforce limits. |
| `max_requests` | integer | `100` | Maximum number of requests allowed within the time window before returning `429 Too Many Requests`. |
| `window_seconds` | integer | `60` | Duration of the sliding time window in seconds. |
| `key_strategy` | enum | `"ip"` | How to identify clients for rate limiting. See below. |
| `key_header` | string | `""` | Header name to use when `key_strategy` is `"header"`. Ignored for other strategies. |
| `cleanup_threshold` | integer | `10000` | Number of tracked client entries before triggering cleanup of expired entries. Increase for high-traffic deployments with many unique clients. |

**Key strategies:**

| Strategy | Behavior |
|---|---|
| `ip` | Rate limits by client IP address. Each unique IP gets its own counter. |
| `header` | Rate limits by the value of a specific request header (set `key_header` to the header name). Useful for rate limiting by tenant or API consumer ID. |
| `token` | Rate limits by the authentication token or API key. Each unique token gets its own counter. |

**Example - 30 requests per minute per IP:**

```yaml
rate_limit:
  enabled: true
  max_requests: 30
  window_seconds: 60
  key_strategy: "ip"
```

---

## Databases

Named database connections. You can define multiple databases and reference them by name in table schemas and endpoints. At least one database is required to use CRUD endpoints.

```yaml
databases:
  main:
    driver: "sqlite"
    url: "sqlite://app.db?mode=rwc"
    min_connections: 1
    max_connections: 10
    auto_migrate: true
    acquire_timeout: 5
```

Each key under `databases` is a unique name you'll use to reference this database elsewhere in the config.

| Field | Type | Default | Description |
|---|---|---|---|
| `driver` | enum | `"sqlite"` | Database driver. One of: `postgres`, `mysql`, `sqlite`. |
| `url` | string | `""` | Connection URL. **Required.** Use `${ENV_VAR}` for secrets. See format examples below. |
| `min_connections` | integer | `1` | Minimum number of connections maintained in the pool. |
| `max_connections` | integer | `10` | Maximum number of connections in the pool. Must be greater than 0 and >= `min_connections`. |
| `auto_migrate` | boolean | `true` | When `true`, Main Serve automatically creates or updates tables (defined in the `tables` section) for this database on startup and on config reload. |
| `allow_destructive` | boolean | `false` | When `true`, auto-migration may drop columns that exist in the database but are no longer present in the YAML config. When `false`, removed columns are logged as warnings but left untouched. **Use with caution - dropped columns lose data irreversibly.** |
| `acquire_timeout` | integer | `5` | Pool connection acquire timeout in seconds. How long to wait for a connection from the pool before returning an error. Increase for databases under heavy load. |

**Connection URL formats:**

| Driver | URL Format |
|---|---|
| `sqlite` | `sqlite://path/to/database.db?mode=rwc` - The `?mode=rwc` flag creates the file if it doesn't exist. |
| `postgres` | `postgres://user:password@host:5432/dbname` |
| `mysql` | `mysql://user:password@host:3306/dbname` |

**Multiple databases example:**

```yaml
databases:
  primary:
    driver: "postgres"
    url: "${PRIMARY_DB_URL}"
    max_connections: 20
  analytics:
    driver: "postgres"
    url: "${ANALYTICS_DB_URL}"
    max_connections: 5
    auto_migrate: false
  cache:
    driver: "sqlite"
    url: "sqlite://cache.db?mode=rwc"
    max_connections: 3
```

### Auto-Migration Behavior

When `auto_migrate` is `true`, Main Serve compares the YAML-defined table schemas against the live database on every startup and config reload:

| Scenario | Action |
|---|---|
| Table does not exist | `CREATE TABLE` with all columns, constraints, and foreign keys. |
| Column in YAML but not in DB | `ALTER TABLE ... ADD COLUMN` - the column is added to the existing table. |
| Column in DB but not in YAML | **If `allow_destructive: true`:** `ALTER TABLE ... DROP COLUMN` - the column is removed and its data is lost. **If `allow_destructive: false` (default):** a warning is logged; the column is left untouched. |
| Column type or constraint mismatch | A warning is logged. No automatic alteration is attempted because cross-driver support is inconsistent. |
| Index does not exist | `CREATE INDEX IF NOT EXISTS` - idempotent. |

**Limitations when adding columns to existing tables:**

- A `NOT NULL` column **must** have a `default` value; otherwise the migration is skipped with a warning (existing rows would have no value).
- Primary key columns cannot be added via ALTER TABLE - a manual migration is required.
- On SQLite, `UNIQUE` constraints cannot be added via ALTER TABLE and are silently omitted.

---

## Table Schemas

Define database tables for auto-migration and CRUD query generation. Each key is the table name.

```yaml
tables:
  users:
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "email"
        type: "varchar"
        nullable: false
        unique: true
        indexed: true
      - name: "created_at"
        type: "timestamptz"
        default: "now()"
    foreign_keys: []
```

| Field | Type | Default | Description |
|---|---|---|---|
| `database` | string | `"main"` | Which named database this table belongs to. Must match a key in the `databases` section. |
| `columns` | list | *(required)* | Column definitions. See [Columns](#columns). |
| `foreign_keys` | list | `[]` | Foreign key constraints. See [Foreign Keys](#foreign-keys). |

### Columns

Each column in the `columns` list defines a database column.

| Field | Type | Default | Description |
|---|---|---|---|
| `name` | string | *(required)* | Column name. Must be unique within the table. |
| `type` | enum | *(required)* | Column data type. See the type table below. |
| `primary_key` | boolean | `false` | Whether this column is part of the primary key. Primary key columns are automatically `NOT NULL`. Every table must have at least one primary key column. |
| `nullable` | boolean | `true` | Whether the column allows NULL values. Primary key columns are always NOT NULL regardless of this setting. |
| `default` | string | `null` | Default SQL expression for the column. This is inserted as a literal SQL expression - use SQL syntax appropriate for your database driver. Examples: `"now()"`, `"CURRENT_TIMESTAMP"`, `"false"`, `"0"`, `"'active'"` (note the inner quotes for string literals). |
| `unique` | boolean | `false` | Whether to add a UNIQUE constraint on this column. |
| `indexed` | boolean | `false` | Whether to create an index on this column. Primary key columns are already indexed by the database, so this has no effect on them. |

**Column types:**

| Type | Description | PostgreSQL | MySQL | SQLite |
|---|---|---|---|---|
| `integer` | Standard integer | `INTEGER` | `INTEGER` | `INTEGER` |
| `bigint` | Large integer | `BIGINT` | `BIGINT` | `BIGINT` |
| `smallint` | Small integer | `SMALLINT` | `SMALLINT` | `SMALLINT` |
| `serial` | Auto-incrementing integer | `SERIAL` | `INTEGER AUTO_INCREMENT` | `INTEGER` (with AUTOINCREMENT on PK) |
| `bigserial` | Auto-incrementing large int | `BIGSERIAL` | `BIGINT AUTO_INCREMENT` | `INTEGER` |
| `text` | Unlimited-length text | `TEXT` | `TEXT` | `TEXT` |
| `varchar` | Variable-length string | `VARCHAR(255)` | `VARCHAR(255)` | `TEXT` |
| `char` | Fixed-length string | `CHAR(255)` | `CHAR(255)` | `TEXT` |
| `boolean` | True/false | `BOOLEAN` | `BOOLEAN` | `INTEGER` |
| `float` | Single-precision float | `REAL` | `REAL` | `REAL` |
| `double` | Double-precision float | `DOUBLE PRECISION` | `DOUBLE PRECISION` | `REAL` |
| `decimal` | Exact decimal | `DECIMAL(10,2)` | `DECIMAL(10,2)` | `REAL` |
| `date` | Date (no time) | `DATE` | `DATE` | `TEXT` |
| `timestamp` | Date and time (no timezone) | `TIMESTAMP` | `DATETIME` | `TEXT` |
| `timestamptz` | Date and time with timezone | `TIMESTAMPTZ` | `DATETIME` | `TEXT` |
| `uuid` | UUID | `UUID` | `CHAR(36)` | `TEXT` |
| `json` | JSON data | `JSON` | `JSON` | `TEXT` |
| `jsonb` | Binary JSON (indexed) | `JSONB` | `JSON` | `TEXT` |
| `blob` | Binary data | `BYTEA` | `BLOB` | `BLOB` |
| `bytea` | Binary data (alias) | `BYTEA` | `BLOB` | `BLOB` |

> **SQLite note:** SQLite has limited type support. Many types (dates, UUIDs, JSON) are stored as TEXT. Booleans are stored as INTEGER (0/1).

### Foreign Keys

Define referential integrity constraints between tables.

```yaml
tables:
  posts:
    database: "main"
    columns:
      - name: "id"
        type: "serial"
        primary_key: true
      - name: "author_id"
        type: "integer"
        nullable: false
    foreign_keys:
      - column: "author_id"
        references_table: "users"
        references_column: "id"
        on_delete: "cascade"
        on_update: "cascade"
```

| Field | Type | Default | Description |
|---|---|---|---|
| `column` | string | *(required)* | Column in this table that holds the foreign key. |
| `references_table` | string | *(required)* | Target table name. |
| `references_column` | string | *(required)* | Target column in the referenced table. |
| `on_delete` | enum | `"restrict"` | Action when the referenced row is deleted. One of: `cascade`, `set_null`, `restrict`, `no_action`. |
| `on_update` | enum | `"restrict"` | Action when the referenced row's key is updated. One of: `cascade`, `set_null`, `restrict`, `no_action`. |

**Foreign key actions:**

| Action | Behavior |
|---|---|
| `cascade` | Automatically delete/update the referencing rows. |
| `set_null` | Set the foreign key column to NULL (column must be nullable). |
| `restrict` | Prevent the delete/update if referencing rows exist. |
| `no_action` | Similar to `restrict`, but checked at the end of the transaction (database-dependent). |

---

## Authentication

Authentication providers are defined globally in the `auth` section and referenced by name in endpoints. Main Serve supports four authentication methods.

### JWT

JSON Web Token authentication. Tokens are validated on each request by checking the signature, expiry, issuer, and audience claims.

```yaml
auth:
  jwt:
    secret: "${JWT_SECRET}"
    algorithm: "HS256"
    issuer: "main-serve"
    audience: ""
    expiry: 3600
    role_claim: "role"
```

| Field | Type | Default | Description |
|---|---|---|---|
| `secret` | string | `""` | Signing key. For HMAC algorithms (HS256/HS384/HS512), this is the shared secret. For RSA/EC algorithms, this is the path to or contents of the public key. **Required.** Use `${ENV_VAR}`. |
| `algorithm` | enum | `"HS256"` | JWT signing algorithm. One of: `HS256`, `HS384`, `HS512`, `RS256`, `RS384`, `RS512`, `ES256`, `ES384`. |
| `issuer` | string | `"main-serve"` | Expected `iss` (issuer) claim. Tokens with a different issuer are rejected. Set to `""` to skip issuer validation. Also used as the `iss` claim when Main Serve creates tokens (e.g., after OAuth2 login). |
| `audience` | string | `""` | Expected `aud` (audience) claim. Tokens with a different audience are rejected. Set to `""` to skip audience validation. |
| `expiry` | integer | `3600` (1 hour) | Token lifetime in seconds. Used when Main Serve creates tokens (e.g., after OAuth2 login). Tokens are set to expire `expiry` seconds after creation. |
| `role_claim` | string | `"role"` | Name of the JWT claim that contains the user's role. This is used for role-based authorization on endpoints. |

**How JWT auth works on a request:**
1. The `Authorization: Bearer <token>` header is checked first.
2. If not found, the cookie specified by `auth.oauth2.cookie_name` (default: `main_serve_token`) is checked - this allows seamless authentication after OAuth2 login.
3. The token is decoded and validated against the configured algorithm, secret, issuer, and audience.
4. The role is extracted from the configured `role_claim` and checked against endpoint `roles`.

### API Key

Simple API key authentication via HTTP header or query parameter.

```yaml
auth:
  api_key:
    location: "header"
    name: "X-API-Key"
    keys:
      - key: "${API_KEY_ADMIN}"
        role: "admin"
      - key: "${API_KEY_READER}"
        role: "reader"
```

| Field | Type | Default | Description |
|---|---|---|---|
| `location` | enum | `"header"` | Where to look for the API key. `"header"` checks request headers; `"query"` checks URL query parameters. |
| `name` | string | `"X-API-Key"` | Header name or query parameter name to extract the key from. |
| `keys` | list | *(required)* | List of valid API keys. Each key can have an associated role. |
| `keys[].key` | string | *(required)* | The API key value. Use `${ENV_VAR}` to avoid storing keys in the config file. |
| `keys[].role` | string | *(optional)* | Role assigned to requests using this key. Used for role-based authorization. |

### HTTP Basic

HTTP Basic authentication with Argon2-hashed passwords.

```yaml
auth:
  basic:
    realm: "main-serve"
    users:
      - username: "admin"
        password_hash: "${ADMIN_PASSWORD_HASH}"
        role: "admin"
      - username: "viewer"
        password_hash: "${VIEWER_PASSWORD_HASH}"
        role: "viewer"
```

| Field | Type | Default | Description |
|---|---|---|---|
| `realm` | string | `"main-serve"` | Realm string returned in the `WWW-Authenticate: Basic realm="..."` response header on authentication failure. |
| `users` | list | *(required)* | List of authorized users. |
| `users[].username` | string | *(required)* | Username for login. |
| `users[].password_hash` | string | *(required)* | Argon2 hash of the password. Use `${ENV_VAR}` for storage. Generate with tools like `argon2` CLI: `echo -n "password" \| argon2 somesalt -id -e`. |
| `users[].role` | string | *(optional)* | Role assigned to this user, used for role-based authorization. |

### OAuth2 / OIDC

OpenID Connect / OAuth2 authentication. Main Serve supports two modes:

**Mode 1 - Token Introspection:** Validates externally-obtained Bearer tokens by calling the provider's userinfo endpoint. Endpoints use `auth: "oauth2"`. The provider returns a JSON object with `sub` (subject) and optional role fields.

**Mode 2 - Authorization Code Flow with PKCE:** Main Serve handles the full login flow: redirects users to the identity provider, exchanges the authorization code for tokens, fetches user info, mints a Main Serve JWT, and sets it as an HttpOnly cookie. This bridges OAuth2 into the JWT auth model, so endpoints with `auth: "jwt"` work seamlessly after login. Requires `auth.jwt` to be configured alongside `auth.oauth2`.

```yaml
auth:
  jwt:
    secret: "${JWT_SECRET}"       # Required for code flow (mints JWTs)

  oauth2:
    provider: "generic"
    authorization_url: "https://idp.example.com/authorize"
    token_url: "https://idp.example.com/token"
    userinfo_url: "https://idp.example.com/userinfo"
    client_id: "${OAUTH2_CLIENT_ID}"
    client_secret: "${OAUTH2_CLIENT_SECRET}"
    scopes:
      - "openid"
      - "profile"
      - "email"
    redirect_url: "http://localhost:8080/_main-serve/oauth2/callback"
    success_url: "/"
    cookie_name: "main_serve_token"
    state_ttl: 300
    max_pending_states: 1000
```

| Field | Type | Default | Description |
|---|---|---|---|
| `provider` | string | `"generic"` | Provider name, used for logging and identification. |
| `authorization_url` | string | `""` | The identity provider's authorization endpoint (login page). **When set**, enables the authorization code flow and registers `/_main-serve/oauth2/authorize` and `/_main-serve/oauth2/callback` endpoints. Leave empty to use token introspection mode only. |
| `token_url` | string | `""` | Token endpoint URL where the authorization code is exchanged for access/ID tokens. **Required when `authorization_url` is set.** |
| `userinfo_url` | string | `""` | OIDC UserInfo endpoint. In introspection mode (Mode 1), this is called with the Bearer token to validate it. In code flow mode (Mode 2), this is called after token exchange to get user info. Must return JSON with a `sub` field and optionally a role field. |
| `client_id` | string | `""` | OAuth2 client ID registered with the provider. Required for code flow. Use `${ENV_VAR}`. |
| `client_secret` | string | `""` | OAuth2 client secret. Use `${ENV_VAR}`. |
| `scopes` | list of strings | `["openid"]` | OAuth2 scopes to request during the authorization code flow. |
| `redirect_url` | string | `""` | The callback URL the identity provider redirects to after authentication. Must match the redirect URI registered with the provider. Typically `http://yourhost/_main-serve/oauth2/callback`. |
| `success_url` | string | `"/"` | URL to redirect the user to after successful OAuth2 login and JWT minting. Can be an absolute URL or a relative path. |
| `cookie_name` | string | `"main_serve_token"` | Name of the HttpOnly cookie set after successful login. Contains the minted JWT. The JWT auth middleware checks this cookie as a fallback when no `Authorization` header is present. |
| `state_ttl` | integer | `300` (5 minutes) | Maximum lifetime in seconds for a pending OAuth2 authorization state. States older than this are expired and rejected during the callback. |
| `max_pending_states` | integer | `1000` | Maximum number of concurrent pending OAuth2 authorization flows. Limits memory usage from abandoned or concurrent authorization attempts. |

**Code flow sequence:**
1. User visits `/_main-serve/oauth2/authorize`
2. Main Serve generates a PKCE code verifier/challenge and random state, stores them, then redirects the user to the IdP's `authorization_url`
3. User authenticates with the IdP
4. IdP redirects to `/_main-serve/oauth2/callback` with an authorization code
5. Main Serve exchanges the code for tokens via `token_url`
6. Main Serve fetches user info from `userinfo_url`
7. Main Serve mints a JWT and sets it as an HttpOnly cookie (`cookie_name`)
8. User is redirected to `success_url`

---

## Endpoints

The core of Main Serve. Each entry in the `endpoints` list defines an HTTP route and its behavior.

**Common fields** (apply to all endpoint types):

| Field | Type | Default | Description |
|---|---|---|---|
| `path` | string | *(required)* | URL path pattern. Supports path parameters (`:id`) and wildcards (`*`). Examples: `/api/users`, `/api/users/:id`, `/static/*`. |
| `methods` | list of enums | *(required)* | HTTP methods this endpoint responds to. Values: `get`, `post`, `put`, `patch`, `delete`, `options`, `head`. |
| `action` | enum | *(required)* | What this endpoint does. One of: `crud`, `proxy`, `static`, `custom_response`. |
| `auth` | string | `"none"` | Authentication method. Must be `"none"` or the name of a provider defined in the `auth` section: `"jwt"`, `"api_key"`, `"basic"`, `"oauth2"`. |
| `roles` | list of strings | *(optional)* | Roles allowed to access this endpoint. If omitted, any authenticated user is allowed. Only meaningful when `auth` is not `"none"`. |
| `cors` | object | *(inherited)* | Per-endpoint CORS override. Same fields as the global `cors` section. If omitted, the global CORS settings apply. |
| `rate_limit` | object | *(inherited)* | Per-endpoint rate limit override. Same fields as the global `rate_limit` section. If omitted, the global rate limit settings apply. |

**Path parameters:**

- `:name` - Captures a URL segment. In CRUD endpoints, `:id` is automatically used as the primary key for single-resource operations (GET, PUT, DELETE on `/api/users/:id`).
- `*` - Matches any trailing path. Used for proxy endpoints (`/api/external/*`) and static file serving (`/static/*`).

### CRUD Endpoints

Automatically map HTTP methods to SQL operations on a defined table.

| HTTP Method | SQL Operation | Description |
|---|---|---|
| GET (collection, e.g., `/api/users`) | SELECT | List records with pagination, filtering, sorting |
| GET (single, e.g., `/api/users/:id`) | SELECT WHERE id = :id | Get one record by primary key |
| POST | INSERT | Create a new record |
| PUT | UPDATE WHERE id = :id | Update an existing record |
| PATCH | UPDATE WHERE id = :id | Partially update a record |
| DELETE | DELETE WHERE id = :id | Delete a record |

```yaml
- path: "/api/users"
  methods: ["get", "post"]
  action: "crud"
  crud:
    table: "users"
    database: "main"
    fields: ["id", "email", "role", "created_at"]
    writable_fields: ["email", "role"]
    pagination:
      enabled: true
      default_page_size: 20
      max_page_size: 100
    filtering:
      enabled: true
      allowed_fields: ["email", "role"]
    sorting:
      enabled: true
      default_field: "created_at"
      default_order: "desc"
      allowed_fields: ["id", "email", "created_at"]
    where_clause: null
    joins: []
    computed_fields: []
```

**CRUD configuration fields:**

| Field | Type | Default | Description |
|---|---|---|---|
| `table` | string | *(required)* | Table name. Must match a key in the `tables` section. |
| `database` | string | table's `database` or `"main"` | Which named database to query. Must match a key in `databases`. |
| `fields` | list of strings | `["*"]` | Columns to include in GET responses. Use `["*"]` for all columns. This controls what data is returned to the client - use it to exclude sensitive fields like `password_hash`. |
| `writable_fields` | list of strings | *(all non-PK)* | Columns that can be set via POST/PUT/PATCH. If omitted, all non-primary-key columns are writable. Use this to prevent clients from setting fields like `created_at` or `id`. |

**Pagination:**

| Field | Type | Default | Description |
|---|---|---|---|
| `pagination.enabled` | boolean | `true` | Enable pagination on list (collection) GET endpoints. |
| `pagination.default_page_size` | integer | `20` | Number of records per page when the client doesn't specify `page_size`. |
| `pagination.max_page_size` | integer | `100` | Maximum value the client can request for `page_size`. Requests above this are clamped. |

When pagination is enabled, clients use query parameters:
- `?page=2` - page number (1-indexed)
- `?page_size=50` - records per page (also accepts `?per_page=50`)

**Filtering:**

| Field | Type | Default | Description |
|---|---|---|---|
| `filtering.enabled` | boolean | `true` | Enable filtering via query parameters. |
| `filtering.allowed_fields` | list of strings | `["*"]` | Which columns can be filtered on. Use `["*"]` to allow filtering on any column. |

When filtering is enabled, clients pass column names as query parameters:
- `?role=admin` - filter where role = "admin"
- `?published=true` - filter where published = true

**Sorting:**

| Field | Type | Default | Description |
|---|---|---|---|
| `sorting.enabled` | boolean | `true` | Enable sorting via query parameters. |
| `sorting.default_field` | string | primary key | Default column to sort by when the client doesn't specify. |
| `sorting.default_order` | enum | `"asc"` | Default sort direction. `"asc"` or `"desc"`. |
| `sorting.allowed_fields` | list of strings | `["*"]` | Which columns can be sorted on. Use `["*"]` to allow sorting on any column. |

When sorting is enabled, clients use query parameters:
- `?sort=created_at` - sort by column
- `?order=desc` - sort direction

**Advanced CRUD features:**

| Field | Type | Default | Description |
|---|---|---|---|
| `where_clause` | string | `null` | Static SQL boolean expression applied to all queries. Example: `"deleted_at IS NULL"` to implement soft deletes. Uses parameterized queries for safety. |
| `joins` | list | `[]` | Table joins for enriching query results. See below. |
| `computed_fields` | list | `[]` | Virtual columns computed from SQL expressions. See below. |

**Joins:**

```yaml
joins:
  - table: "organizations"
    on: "users.org_id = organizations.id"
    type: "left"                # inner, left, right
    fields:
      - "organizations.name as org_name"
```

| Field | Type | Description |
|---|---|---|
| `table` | string | Table to join with. |
| `on` | string | JOIN condition (SQL expression). |
| `type` | enum | Join type: `inner`, `left`, `right`. |
| `fields` | list of strings | Columns to include from the joined table. Use `AS` aliases for clarity. |

**Computed fields:**

```yaml
computed_fields:
  - name: "full_name"
    expression: "first_name || ' ' || last_name"
```

| Field | Type | Description |
|---|---|---|
| `name` | string | Alias for the computed column in the response. |
| `expression` | string | SQL expression to compute the value. |

### Proxy Endpoints

Forward requests to an upstream server. The request path, headers, and body are forwarded; the upstream response is returned to the client.

```yaml
- path: "/api/external/*"
  methods: ["get", "post", "put", "delete"]
  action: "proxy"
  proxy:
    upstream: "https://api.example.com"
    path_rewrite:
      strip_prefix: "/api/external"
      add_prefix: "/v2"
    headers:
      X-Forwarded-For: "client"
      Authorization: "Bearer ${UPSTREAM_TOKEN}"
    timeouts:
      connect: 5
      read: 30
      total: 60
    max_response_size: 268435456
```

| Field | Type | Default | Description |
|---|---|---|---|
| `upstream` | string | *(required)* | Base URL of the upstream server to forward requests to. |
| `path_rewrite.strip_prefix` | string | `""` | Remove this prefix from the request path before forwarding. For example, with `strip_prefix: "/api/external"`, a request to `/api/external/users` is forwarded as `/users`. |
| `path_rewrite.add_prefix` | string | `""` | Prepend this prefix to the path after stripping. Continuing the example, with `add_prefix: "/v2"`, the forwarded path becomes `/v2/users`. |
| `headers` | map (string -> string) | `{}` | Extra headers to add to the upstream request. Values support `${ENV_VAR}` interpolation. |
| `timeouts.connect` | integer | `5` | Connection timeout in seconds. How long to wait for the TCP connection to the upstream. |
| `timeouts.read` | integer | `30` | Read timeout in seconds. How long to wait for the upstream to send response data. |
| `timeouts.total` | integer | `60` | Total timeout in seconds. Maximum total time for the entire proxied request. |
| `max_response_size` | integer | `268435456` (256 MiB) | Maximum upstream response body size in bytes. Responses larger than this are rejected with an error. Set to `0` to disable the limit. |

### Static File Endpoints

Serve files from a directory on disk.

```yaml
- path: "/static/*"
  methods: ["get"]
  action: "static"
  static_files:
    root: "./public"
    index: "index.html"
    directory_listing: false
    cache_max_age: 3600
    spa_fallback: false
```

| Field | Type | Default | Description |
|---|---|---|---|
| `root` | string | *(required)* | Root directory for static files. Can be relative (to the working directory) or absolute. |
| `index` | string | `"index.html"` | Default file to serve when a directory is requested. |
| `directory_listing` | boolean | `false` | Show a listing of directory contents when no index file is found. **Security note:** be cautious enabling this in production. |
| `cache_max_age` | integer | `3600` (1 hour) | Value for the `Cache-Control: max-age=` response header, in seconds. Set to `0` to disable client-side caching. |
| `spa_fallback` | boolean | `false` | When `true`, requests for paths that don't match a real file return the `index` file instead of 404. Essential for single-page applications with client-side routing (React, Vue, Angular, etc.). |

### Custom Response Endpoints

Return a fixed, static response. Useful for health checks, version info, or mock endpoints.

```yaml
- path: "/api/info"
  methods: ["get"]
  action: "custom_response"
  custom_response:
    status: 200
    content_type: "application/json"
    body: '{"name": "My App", "version": "1.0.0"}'
    headers:
      X-Custom: "hello"
```

| Field | Type | Default | Description |
|---|---|---|---|
| `status` | integer | `200` | HTTP status code for the response. |
| `content_type` | string | `"application/json"` | Value for the `Content-Type` response header. |
| `body` | string | `""` | Response body content. For JSON responses, use a JSON string. |
| `headers` | map (string -> string) | `{}` | Additional response headers to include. |

---

## System Endpoints

Main Serve registers these endpoints automatically. They are always available regardless of your configuration.

### Health Check

```
GET /_main-serve/health
```

Returns `200 OK` with a JSON body:

```json
{
  "status": "healthy"
}
```

When databases are configured, includes their connectivity status. No authentication required.

> **Note:** It is recommended that the health endpoint not be publicly exposed.

### Hot Reload

```
POST /_main-serve/reload
Authorization: Bearer <ADMIN_TOKEN>
```

Triggers a full re-read and re-application of the YAML configuration file. The admin token must be set via the `MAIN_SERVE_ADMIN_TOKEN` environment variable or the `--admin-token` CLI flag.

**Reload process:**
1. The new YAML file is parsed and fully validated.
2. If validation fails, the reload is rejected - the running configuration is **unchanged**.
3. If validation passes, the config is atomically swapped, the router is rebuilt, and database pools are updated as needed.
4. In-flight requests complete against the old config. New requests use the new config immediately.
5. The TCP listener stays open - no connections are dropped.

Returns a success response with a summary, or an error with details about what failed.

### OAuth2 Endpoints

These are registered automatically when `auth.oauth2.authorization_url` is set:

```
GET  /_main-serve/oauth2/authorize   - Redirects to the identity provider
GET  /_main-serve/oauth2/callback    - Handles the IdP callback
```

No authentication required (they implement the authentication flow itself).

---

## CLI Reference

```
main-serve [OPTIONS]

Options:
  -c, --config <PATH>    Path to YAML config file
                         [default: ./config/config.yaml, then /etc/main-serve/config.yaml]
  --admin-token <TOKEN>  Admin token for reload endpoint
                         (overrides MAIN_SERVE_ADMIN_TOKEN env var)
  --validate             Validate config and exit without starting the server
  --dry-run              Parse config, print resolved endpoints, and exit
  -h, --help             Print help
  -V, --version          Print version
```

**`--validate`** is useful in CI/CD pipelines to catch config errors before deployment.

**`--dry-run`** shows which endpoints would be registered, helping you verify path patterns and method mappings.

---

## Minimal Config

The smallest possible configuration that does something useful:

```yaml
server:
  port: 8080

databases:
  main:
    driver: "sqlite"
    url: "sqlite://data.db?mode=rwc"

tables:
  items:
    columns:
      - name: "id"
        type: "integer"
        primary_key: true
      - name: "value"
        type: "text"

endpoints:
  - path: "/api/items"
    methods: ["get", "post"]
    action: "crud"
    crud:
      table: "items"
    auth: "none"
```

Every field not specified uses its default value. The `server.host` defaults to `127.0.0.1`, `max_connections` defaults to `10`, pagination and sorting are enabled by default, and so on.

# Quickstart Guide

This guide will take you from zero to a working Main Serve instance in minutes. By the end, you'll have a REST API running with a SQLite database, auto-generated CRUD endpoints, and authentication - all without writing a single line of code.

---

## Prerequisites

- **Rust** (latest stable): [Install via rustup](https://rustup.rs/) or with your platform's package manager
- **Git**: To clone the repository

No external databases are required for this guide - we'll use SQLite, which is built in.

---

## 1. Build Main Serve

```bash
git clone https://github.com/dekoding/main-serve.git
cd main-serve
cargo build --release
```

The compiled binary will be at `./target/release/main-serve`.

---

## 2. Your First Config

Create a file called `my-config.yaml`:

```yaml
server:
  host: "127.0.0.1"
  port: 8080

databases:
  main:
    driver: "sqlite"
    url: "sqlite://myapp.db?mode=rwc"
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
      - name: "created_at"
        type: "timestamp"
        default: "CURRENT_TIMESTAMP"

endpoints:
  # Public: anyone can read
  - path: "/api/todos"
    methods: ["get"]
    action: "crud"
    crud:
      table: "todos"
      database: "main"
      fields: ["id", "title", "done", "created_at"]
    auth: "none"

  # Protected: need API key to create
  - path: "/api/todos"
    methods: ["post"]
    action: "crud"
    crud:
      table: "todos"
      database: "main"
      writable_fields: ["title", "done"]
    auth: "api_key"
    roles: ["admin"]

  # Protected: need API key to modify/delete
  - path: "/api/todos/{id}"
    methods: ["get"]
    action: "crud"
    crud:
      table: "todos"
      database: "main"
      fields: ["id", "title", "done", "created_at"]
    auth: "none"

  - path: "/api/todos/{id}"
    methods: ["put", "delete"]
    action: "crud"
    crud:
      table: "todos"
      database: "main"
      writable_fields: ["title", "done"]
    auth: "api_key"
    roles: ["admin"]
```

Now reload the config without restarting:

```bash
curl -X POST http://localhost:8080/_main-serve/reload \
  -H "Authorization: Bearer my-secret"
```

Test that authentication works:

```bash
# This should fail with 401:
curl -X POST http://localhost:8080/api/todos \
  -H "Content-Type: application/json" \
  -d '{"title": "Unauthorized", "done": false}'

# This should succeed:
curl -X POST http://localhost:8080/api/todos \
  -H "Content-Type: application/json" \
  -H "X-API-Key: my-demo-key" \
  -d '{"title": "Authorized!", "done": false}'
```

---

## 3. Run Main Serve

```bash
MAIN_SERVE_ADMIN_TOKEN=my-secret ./target/release/main-serve -c my-config.yaml
```

This starts the server on `http://localhost:8080`. The admin token is used for the hot reload endpoint.

## 4. Test the API

```bash
# List todos (public)
curl http://localhost:8080/api/todos

# Create a todo (requires API key from auth.api_key.keys)
curl -X POST http://localhost:8080/api/todos \
  -H "Content-Type: application/json" \
  -H "X-API-Key: my-demo-key" \
  -d '{"title": "Learn Rust", "done": false}'

# Health check
curl http://localhost:8080/_main-serve/health
```

## 5. Enhance With Pagination, Sorting & Filtering

The quickstart config demonstrates all the essentials. Now enhance with query features:

```yaml
  - path: "/api/todos"
    methods: ["get"]
    action: "crud"
    crud:
      table: "todos"
      database: "main"
      fields: ["id", "title", "done", "created_at"]
      pagination:
        enabled: true
        default_page_size: 10
        max_page_size: 50
      sorting:
        enabled: true
        default_field: "created_at"
        default_order: "desc"
      filtering:
        enabled: true
        allowed_fields: ["done", "title"]
    auth: "none"
```

Now you can use query parameters:

```bash
# Paginate
curl "http://localhost:8080/api/todos?page=1&page_size=5"

# Sort
curl "http://localhost:8080/api/todos?sort=title&order=asc"

# Filter
curl "http://localhost:8080/api/todos?done=false"
```

---

## 6. Serve Static Files

Add a static file endpoint to host a frontend:

```yaml
stores:
  assets:
    backend: "native"
    root: "./public"

endpoints:
  - path: "/static/*"
    methods: ["get"]
    action: "static_files"
    static_files:
      storage: "assets"
      index: "index.html"
      cache_max_age: 3600
    auth: "none"

  - path: "/app/*"
    methods: ["get"]
    action: "spa_host"
    spa_host:
      storage: "assets"
      index: "index.html"
      cache_max_age: 0
    auth: "none"
```

Create a `public/` directory and place your frontend files there. The SPA host endpoint serves `index.html` for any path that doesn't match a real file - perfect for client-side routing.

---

## 7. Set Up a Reverse Proxy

Forward requests to an upstream API:

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
        Authorization: "Bearer ${UPSTREAM_TOKEN}"
      timeouts:
        connect: 5
        read: 30
        total: 60
    auth: "none"
```

A request to `/api/external/users` will be forwarded to `https://api.example.com/v2/users`.

---

## 8. Environment Variables for Secrets

Never put secrets directly in your YAML. Use `${ENV_VAR}` interpolation:

```yaml
databases:
  main:
    url: "${DATABASE_URL}"

auth:
  jwt:
    secret: "${JWT_SECRET}"
```

With defaults as fallback:

```yaml
auth:
  api_key:
    keys:
      - key: "${API_KEY:-dev-key-12345}"
```

If `API_KEY` is not set, it falls back to `dev-key-12345`. Without a default (`${API_KEY}`), Main Serve will error on startup if the variable is missing.

---

## 9. Validate Without Running

Check your config for errors without starting the server:

```bash
./target/release/main-serve -c my-config.yaml --validate
```

Or preview what endpoints would be created:

```bash
./target/release/main-serve -c my-config.yaml --dry-run
```

---

## Next Steps

- Read the full [Configuration Reference](CONFIGURATION.md) for every field and option
- See the [spec.yaml](../config/spec.yaml) for the annotated configuration specification
- Check out the [demo config](../config/templates/default.yaml) for a working example
- Read the [Manifesto](MANIFESTO.md) to understand the project's philosophy

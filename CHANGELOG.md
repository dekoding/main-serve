# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-04-16

### Added

- YAML-configured web server with hot reload via `POST /_main-serve/reload`
- Server settings: bind address, port, TLS (rustls), worker threads, keep-alive, request body limits
- Multiple named database backends (PostgreSQL, MySQL, SQLite) with connection pooling
- Auto-migration from YAML-defined table schemas (columns, foreign keys, indexes)
- CRUD endpoint generation: GET/POST/PUT/PATCH/DELETE mapped to SELECT/INSERT/UPDATE/DELETE
- Pagination, filtering, and sorting for list endpoints
- JOIN support and computed fields in CRUD queries
- Reverse proxy endpoints with path rewriting, header injection, and configurable timeouts
- Static file serving with directory listing, cache headers, and SPA fallback
- Custom response endpoints with configurable status, headers, and body
- Global and per-endpoint CORS configuration
- Global and per-endpoint rate limiting (by IP, header, or token)
- Authentication providers: JWT, API key (header/query), HTTP Basic, OAuth2/OIDC
- Role-based authorization per endpoint
- Environment variable interpolation (`${ENV_VAR}` and `${ENV_VAR:-default}`) in YAML values
- `$include` directive for splitting config across multiple files (with glob support)
- Structured logging with JSON or pretty format, configurable per-request body logging
- Request tracing with UUID request IDs (`X-Request-Id` header)
- Graceful shutdown with configurable timeout
- Health check endpoint at `GET /_main-serve/health`
- CLI with `--config`, `--admin-token`, `--validate`, and `--dry-run` flags
- Linux packaging: .deb (cargo-deb), .rpm (cargo-generate-rpm), Arch PKGBUILD
- systemd service unit with security hardening

[0.1.0]: https://github.com/dekoding/main-serve/releases/tag/v0.1.0

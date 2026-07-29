# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.3.x   | :white_check_mark: |
| 0.2.x   | :white_check_mark: |

## Reporting a Vulnerability

If you discover a security vulnerability in Main Serve, please report it responsibly.

**Do NOT open a public GitHub issue for security vulnerabilities.**

Instead, please email **dekoding@damonkaswell.com** with:

1. A description of the vulnerability
2. Steps to reproduce the issue
3. The potential impact
4. Any suggested fix (optional)

You should receive an acknowledgment within **48 hours**. I will work with you to understand the issue and coordinate a fix and disclosure timeline.

## Security Considerations

Main Serve handles several security-sensitive areas:

- **Authentication** (JWT, API keys, Basic auth, OAuth2)
- **Database access** (SQL query building from user-defined config)
- **Reverse proxying** (upstream request forwarding)
- **Environment variable interpolation** (secret injection)

Contributions touching these areas receive extra scrutiny during review.

## Best Practices for Users

- Never store secrets directly in YAML config files - use `${ENV_VAR}` interpolation
- Set a strong `MAIN_SERVE_ADMIN_TOKEN` for the reload endpoint
- Run Main Serve as a dedicated system user (the systemd unit does this by default)
- Use TLS in production
- Restrict the bind address to `127.0.0.1` if behind a reverse proxy

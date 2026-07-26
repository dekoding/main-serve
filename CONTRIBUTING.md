# Contributing to Main Serve

Welcome! I'm really glad you're interested in contributing to the Main Serve project. I just have a few standards I'd like you to follow with your pull requests.

## Commit Message Format

This project follows [conventional commits](https://www.conventionalcommits.org/). All commit messages must follow this format:

```
<type>(<scope>): <description>
```

### Valid Types

- `feat` - New feature
- `fix` - Bug fix
- `docs` - Documentation changes
- `style` - Formatting, semicolons, etc
- `refactor` - Code restructuring
- `perf` - Performance improvements
- `test` - Adding or updating tests
- `build` - Build system changes
- `ci` - CI configuration
- `chore` - Maintenance tasks
- `revert` - Reverting previous commits

### Examples of High Quality Commit Messages

```
feat(auth): add JWT token validation
fix(config): handle missing environment variables
docs(readme): update installation instructions
refactor(server): extract router configuration
test(api): add integration tests for CRUD endpoints
```

The commit message body and footer are optional but should follow conventional commits guidelines if used. **DO NOT** include co-authorship signatures from AI tools. You must take responsibility for all code you submit.

## General Guidelines

- PRs should be single-purpose and focused. If you submit a PR that rewrites large swaths of the codebase, I'm probably rejecting it.
- All PRs must be created from the `develop` branch, using feature branches (e.g., `feat/my-awesome-feature`).
- I try to follow [DRY](https://en.wikipedia.org/wiki/Don%27t_repeat_yourself) principles. Your PR shouldn't reinvent any wheels that are already present in the project.
- **No AI slop**. I'm not opposed to the use of AI to *assist* you, but if you haven't read and understood your own code, it's going to be obvious and your PR will be rejected.

## Rust Best Practices

This is a **Rust** project and tries to follow typical Rust patterns. Please adhere to the following when submitting code:

### Code Style & Structure

- Run `cargo fmt` before committing. All code should be formatted with the default `rustfmt` settings.
- Run `cargo clippy` and address all warnings. PRs with clippy warnings will be sent back for fixes.
- Follow the convention of `struct` followed by its `impl` block in the same file.
- Keep modules focused - one responsibility per module. If a file is getting long, consider splitting it.

### Error Handling

- Use `thiserror` for defining error types in library code. Only use `anyhow` in `main.rs` / CLI code.
- **No `unwrap()` or `expect()` in library or handler code.** Propagate errors with `?` instead. The only acceptable exception is in tests.
- HTTP error responses must always be JSON: `{ "error": { "code": "...", "message": "..." } }`.

### Async & Concurrency

- All I/O must be non-blocking. Use `tokio::fs` instead of `std::fs`, `tokio::net` instead of `std::net`, etc.
- **No `unsafe` code** unless absolutely unavoidable, and if so, justify it thoroughly in comments.
- Prefer `tokio::sync::RwLock` over `Mutex` when readers outnumber writers. Prefer atomics when a lock isn't needed at all.
- Use `tokio::sync::mpsc` or `tokio::sync::watch` for channel communication - never `std::sync` channels.

### Types & Serialization

- Use enums where a fixed set of values is expected (HTTP methods, database drivers, auth types) - never raw `String`.
- Config structs should implement `Default` with production-safe defaults and use `#[serde(default)]` so partial configs work.
- All public types and functions should have doc comments (`///`).

### Security

- If you are adding features that interact with the database layer, never build SQL queries via string concatenation. Use parameterized queries or the query builder in `src/db/query.rs`.
- Never store secrets in YAML or source code. Use `${ENV_VAR}` interpolation for any sensitive values.
- Validate all external inputs at system boundaries (user requests, config loading, environment variables).

### Testing

- Write tests for new functionality. Use `#[tokio::test]` for async tests.
- Integration tests go in the `tests/` directory. Unit tests go in the module they test, inside a `#[cfg(test)] mod tests` block.
- Tests should be self-contained - don't rely on external services or state from other tests.

### How you can help

The easiest way to contribute is to help with documentation. Almost all documentation is currently in the [README](./README.md), the [behavioral spec](./docs/spec.md), the [JSON Schema](./config/main-serve.schema.json), and the code itself. All other forms of documentation are a work in progress, and you could contribute immediately by helping to update things like the [quickstart guide](./docs/QUICKSTART.md) or [configuration guide](./docs/CONFIGURATION.md). The project would also benefit from a collection of high-quality server configurations to use as templates.

There are also a number of features that an expert Rust developer could assist with immediately:

- Support for more database backends (such as NoSQL databases).
- Support for websockets.
- File upload/download support.

If you have an idea you'd like to see become part of Main Serve, but aren't a developer yourself, fill out a feature request [here](https://github.com/dekoding/main-serve/issues).

Again, welcome, and happy contributing!

*Damon Kaswell (dekoding)*

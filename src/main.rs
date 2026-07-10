use std::net::SocketAddr;
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::{env, fmt, time::Duration};

use clap::Parser;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing_subscriber::EnvFilter;

use main_serve::config::load_config;
use main_serve::config::types::{LogFormat, RevocationStoreType};
use main_serve::db::migration::{ensure_media_columns, run_migrations};
use main_serve::db::pool::{close_pools, create_pools};
use main_serve::server::state::{
    DatabaseRevocationStore, InMemoryRevocationStore, RevocationStoreImpl,
};
use main_serve::server::{AppState, build_router, build_tls_acceptor};

/// Main Serve - a high-performance, YAML-configured web server.
#[derive(Parser)]
#[command(name = "main-serve", version, about)]
/// item
struct Cli {
    /// Path to YAML config file.
    ///
    /// If not specified, checks `$HOME/.config/main-serve/config.yaml` then
    /// `/etc/main-serve/config.yaml`.
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Admin token for the reload endpoint (overrides `MAIN_SERVE_ADMIN_TOKEN` env var).
    #[arg(long, env = "MAIN_SERVE_ADMIN_TOKEN", default_value = "")]
    admin_token: String,

    /// Validate config and exit without starting the server.
    #[arg(long)]
    validate: bool,

    /// Parse config, print resolved endpoints, and exit.
    #[arg(long)]
    dry_run: bool,
}

impl fmt::Debug for Cli {
    /// item
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cli")
            .field("config", &self.config)
            .field("admin_token", &"[REDACTED]")
            .field("validate", &self.validate)
            .field("dry_run", &self.dry_run)
            .finish()
    }
}

/// item
fn main() {
    let cli = Cli::parse();

    // Resolve config path: explicit CLI flag -> $HOME/.config/main-serve/config.yaml -> /etc/main-serve/config.yaml
    let config_path = resolve_config_path(cli.config.as_ref());

    // Load config first (before tracing init) so we can use logging settings.
    // We can't log config errors with tracing yet, so use eprintln.
    let config = match load_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load config: {e}");
            process::exit(1);
        }
    };

    // Initialize tracing using config values.
    // RUST_LOG env var takes precedence over the config file level.
    let filter_str = match config.logging.level {
        main_serve::config::types::LogLevel::Trace => "trace",
        main_serve::config::types::LogLevel::Debug => "debug",
        main_serve::config::types::LogLevel::Info => "info",
        main_serve::config::types::LogLevel::Warn => "warn",
        main_serve::config::types::LogLevel::Error => "error",
    };
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter_str));

    match config.logging.format {
        LogFormat::Json => {
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(env_filter)
                .init();
        }
        LogFormat::Pretty => {
            tracing_subscriber::fmt().with_env_filter(env_filter).init();
        }
    }

    // Build the tokio runtime with configurable worker threads.
    let mut runtime_builder = tokio::runtime::Builder::new_multi_thread();
    runtime_builder.enable_all();
    if config.server.workers > 0 {
        runtime_builder.worker_threads(config.server.workers);
    }
    let runtime = runtime_builder.build().unwrap_or_else(|e| {
        tracing::error!("Failed to build tokio runtime: {e}");
        process::exit(1);
    });

    runtime.block_on(async_main(cli, config, config_path));
}

/// Resolve the config file path using priority order:
/// 1. Explicit CLI flag (if provided)
/// 2. `$HOME/.config/main-serve/config.yaml` (home directory config)
/// 3. `/etc/main-serve/config.yaml` (system install)
fn resolve_config_path(cli_path: Option<&PathBuf>) -> PathBuf {
    if let Some(explicit) = cli_path {
        return explicit.clone();
    }

    if let Ok(home) = env::var("HOME") {
        let local = PathBuf::from(format!("{home}/.config/main-serve/config.yaml"));
        if local.exists() {
            return local;
        }
    }

    let system = PathBuf::from("/etc/main-serve/config.yaml");
    if system.exists() {
        return system;
    }

    // Default to home directory config path so the error message is helpful.
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(format!("{home}/.config/main-serve/config.yaml"));
    }
    PathBuf::from("$HOME/.config/main-serve/config.yaml")
}

async fn async_main(cli: Cli, config: main_serve::config::AppConfig, config_path: PathBuf) {
    // --validate: just validate and exit.
    if cli.validate {
        tracing::info!("Configuration is valid.");
        process::exit(0);
    }

    // --dry-run: print resolved endpoints and exit.
    if cli.dry_run {
        tracing::info!("Resolved configuration:");
        tracing::info!("  Server: {}:{}", config.server.host, config.server.port);
        tracing::info!("  Databases: {}", config.databases.len());
        tracing::info!("  Tables: {}", config.tables.len());
        tracing::info!("  Stores: {}", config.stores.len());
        tracing::info!("  Endpoints:");
        for ep in &config.endpoints {
            let methods: Vec<String> = ep.methods.iter().map(|m| format!("{m:?}")).collect();
            tracing::info!(
                "    {} [{}] -> {:?}",
                ep.path,
                methods.join(", "),
                ep.action
            );
        }
        process::exit(0);
    }

    // Ensure admin token is set.
    if cli.admin_token.is_empty() {
        tracing::warn!(
            "No admin token configured. The reload endpoint will reject all requests. \
             Set MAIN_SERVE_ADMIN_TOKEN or use --admin-token."
        );
    }

    let host = config.server.host.clone();
    let port = config.server.port;
    let shutdown_timeout = config.server.shutdown_timeout;

    // Build shared state and router.
    let state = match AppState::new(config, config_path, cli.admin_token.clone()).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to initialize application state: {e}");
            process::exit(1);
        }
    };

    // Prepare revocation cleanup handle (declared early so it's in scope for serve_plain/serve_tls).
    let mut rev_cleanup_handle: Option<(oneshot::Sender<()>, tokio::task::JoinSet<()>)> = None;

    // Create database pools, run migrations, and set up revocation store.
    if !state.config.read().await.databases.is_empty() {
        let config_ref = state.config.read().await;
        let databases = config_ref.databases.clone();
        let tables = config_ref.tables.clone();
        let revocation_config = config_ref
            .auth
            .jwt
            .as_ref()
            .and_then(|j| j.revocation.clone());
        drop(config_ref);

        // Create database pools.
        let pools = match create_pools(&databases).await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Failed to create database pools: {e}");
                process::exit(1);
            }
        };

        // Create revocation tables and store if revocation is configured.
        let revocation_store = if let Some(ref rev_config) = revocation_config {
            let store_type = rev_config.store;
            match store_type {
                RevocationStoreType::InMemory => Some(RevocationStoreImpl::InMemory(Arc::new(
                    InMemoryRevocationStore::default(),
                ))),
                RevocationStoreType::Database => {
                    // Create the revocation table in all pools.
                    if let Err(e) =
                        main_serve::db::migration::create_revocation_tables(&pools).await
                    {
                        tracing::error!("Failed to create revocation table: {e}");
                        close_pools(&pools).await;
                        process::exit(1);
                    }

                    // Use the first available pool for the database revocation store.
                    if let Some(pool) = pools.values().next().cloned() {
                        let table_name = rev_config
                            .db_table
                            .clone()
                            .unwrap_or_else(|| "token_blacklist".to_string());
                        let db_store = DatabaseRevocationStore::new(pool, table_name);
                        Some(RevocationStoreImpl::Database(Arc::new(db_store)))
                    } else {
                        tracing::warn!("No database pool available for revocation store");
                        None
                    }
                }
            }
        } else {
            None
        };

        // Run application migrations.
        if let Err(e) = run_migrations(&tables, &pools, &databases).await {
            tracing::error!("Migration failed: {e}");
            close_pools(&pools).await;
            process::exit(1);
        }

        // Ensure media-specific columns (like file_path) exist on media tables.
        {
            let config_ref = state.config.read().await;
            let endpoints = config_ref.endpoints.clone();
            drop(config_ref);
            if let Err(e) = ensure_media_columns(&endpoints, &pools).await {
                tracing::error!("Failed to ensure media columns: {e}");
                close_pools(&pools).await;
                process::exit(1);
            }
        }

        // Set pools on state.
        {
            let mut pool_lock = state.db_pools.write().await;
            *pool_lock = pools;
        }

        // Set revocation store on state (using OnceLock).
        if let Some(store) = revocation_store
            && let Err(e) = state.set_revocation_store(store)
        {
            tracing::error!("Failed to set revocation store: {e}");
            process::exit(1);
        }

        // Spawn a cleanup task for the revocation store (database variant only).
        if let Some(ref rev_config) = revocation_config
            && let Some(interval_secs) = rev_config.cleanup_interval_secs
            && let Some(store) = state.revocation_store.get()
        {
            let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
            let store_clone = store.clone();
            let mut handle = tokio::task::JoinSet::new();
            handle.spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                            if let Err(e) = store_clone.cleanup_expired().await {
                                tracing::warn!("Revocation store cleanup failed: {e}");
                            }
                        }
                        _ = &mut shutdown_rx => break,
                    }
                }
            });
            rev_cleanup_handle = Some((shutdown_tx, handle));
        }
    }

    let config_guard = state.config.read().await;
    let app = build_router(&config_guard, state.clone()).await;
    drop(config_guard);

    // Bind the TCP listener.
    let addr: SocketAddr = format!("{host}:{port}").parse().unwrap_or_else(|e| {
        tracing::error!("Invalid bind address '{host}:{port}': {e}");
        process::exit(1);
    });

    let listener = TcpListener::bind(addr).await.unwrap_or_else(|e| {
        tracing::error!("Failed to bind to {addr}: {e}");
        process::exit(1);
    });

    // Read keep-alive and TLS config.
    let config_read = state.config.read().await;
    let keep_alive = config_read.server.keep_alive;
    let tls_config = config_read.server.tls.clone();
    drop(config_read);

    let local_addr = listener.local_addr().expect("local addr");
    if let Some(ref tls) = tls_config {
        // TLS mode: use tokio-rustls acceptor.
        let acceptor = match build_tls_acceptor(tls) {
            Ok(a) => a,
            Err(e) => {
                tracing::error!("Failed to configure TLS: {e}");
                process::exit(1);
            }
        };

        tracing::info!("Main Serve listening on https://{local_addr}");

        serve_tls(
            listener,
            acceptor,
            app,
            shutdown_timeout,
            keep_alive,
            rev_cleanup_handle,
        )
        .await;
    } else {
        // Plain HTTP mode.
        tracing::info!("Main Serve listening on http://{local_addr}");

        serve_plain(
            listener,
            app,
            shutdown_timeout,
            keep_alive,
            rev_cleanup_handle,
        )
        .await;
    }

    // Drain database pools on shutdown.
    {
        let pools = state.db_pools.read().await;
        close_pools(&pools).await;
    }

    tracing::info!("Server shut down gracefully.");
}

/// Wait for SIGINT or SIGTERM, then allow a grace period for in-flight requests.
async fn shutdown_signal(timeout_secs: u64) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for Ctrl+C");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to listen for SIGTERM")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {
            tracing::info!("Received SIGINT, starting graceful shutdown (timeout: {timeout_secs}s)...");
        }
        () = terminate => {
            tracing::info!("Received SIGTERM, starting graceful shutdown (timeout: {timeout_secs}s)...");
        }
    }
}

/// Build a hyper `auto::Builder` with keep-alive configured from the YAML spec.
///
/// `keep_alive == 0` disables HTTP/1 keep-alive entirely.
/// `keep_alive > 0` enables keep-alive and sets a `header_read_timeout` equal to
/// the configured value, which controls how long idle keep-alive connections wait
/// for the next request before being closed.
fn build_http_builder(
    keep_alive: u64,
) -> hyper_util::server::conn::auto::Builder<hyper_util::rt::tokio::TokioExecutor> {
    use hyper_util::rt::tokio::{TokioExecutor, TokioTimer};
    use hyper_util::server::conn::auto::Builder;

    let mut builder = Builder::new(TokioExecutor::new());
    if keep_alive == 0 {
        builder.http1().keep_alive(false);
    } else {
        builder
            .http1()
            .keep_alive(true)
            .timer(TokioTimer::new())
            .header_read_timeout(std::time::Duration::from_secs(keep_alive));
    }
    builder
}

/// Serve plain HTTP using hyper's connection builder directly.
///
/// This bypasses `axum::serve` so we can configure HTTP/1 keep-alive timeouts
/// via hyper's `header_read_timeout`.
async fn serve_plain(
    listener: TcpListener,
    app: axum::Router,
    shutdown_timeout: u64,
    keep_alive: u64,
    rev_cleanup: Option<(oneshot::Sender<()>, tokio::task::JoinSet<()>)>,
) {
    let shutdown = shutdown_signal(shutdown_timeout);
    tokio::pin!(shutdown);

    let builder = build_http_builder(keep_alive);
    let mut tasks = tokio::task::JoinSet::new();

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (tcp_stream, remote_addr) = match result {
                    Ok(conn) => conn,
                    Err(e) => {
                        tracing::warn!("Failed to accept TCP connection: {e}");
                        continue;
                    }
                };

                let builder = builder.clone();
                let app = app.clone();

                tasks.spawn(async move {
                    handle_connection(tcp_stream, remote_addr, builder, app).await;
                });
            }
            () = &mut shutdown => {
                tracing::info!("Stopping listener...");
                break;
            }
        }
    }

    // Await all in-flight connections with a timeout.
    let conn_timeout = std::time::Duration::from_secs(5);
    if tasks.is_empty() {
        tracing::info!("Server shut down gracefully.");
    } else {
        let start = std::time::Instant::now();
        loop {
            if tasks.is_empty() {
                tracing::info!("Server shut down gracefully.");
                break;
            }
            if start.elapsed() >= conn_timeout {
                tracing::warn!(
                    "Shutdown timed out after {:?}, {} connection(s) forcibly closed",
                    conn_timeout,
                    tasks.len()
                );
                break;
            }
            let remaining = conn_timeout.saturating_sub(start.elapsed());
            if tokio::time::timeout(remaining, tasks.join_next())
                .await
                .is_err()
            {
                if tasks.is_empty() {
                    tracing::info!("Server shut down gracefully.");
                } else {
                    tracing::warn!(
                        "Shutdown timed out after {:?}, {} connection(s) forcibly closed",
                        conn_timeout,
                        tasks.len()
                    );
                }
                break;
            }
        }
    }

    // Signal revocation cleanup to stop.
    if let Some((tx, _)) = rev_cleanup {
        let _ = tx.send(());
    }
}

/// Serve HTTPS using tokio-rustls TLS acceptor.
///
/// Accepts TLS-wrapped TCP connections in a loop and hands each to hyper
/// for HTTP processing. Shuts down gracefully on signal.
async fn serve_tls(
    listener: TcpListener,
    acceptor: tokio_rustls::TlsAcceptor,
    app: axum::Router,
    shutdown_timeout: u64,
    keep_alive: u64,
    rev_cleanup: Option<(oneshot::Sender<()>, tokio::task::JoinSet<()>)>,
) {
    let shutdown = shutdown_signal(shutdown_timeout);
    tokio::pin!(shutdown);

    let builder = build_http_builder(keep_alive);
    let mut tasks = tokio::task::JoinSet::new();

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (tcp_stream, remote_addr) = match result {
                    Ok(conn) => conn,
                    Err(e) => {
                        tracing::warn!("Failed to accept TCP connection: {e}");
                        continue;
                    }
                };

                let acceptor = acceptor.clone();
                let builder = builder.clone();
                let app = app.clone();

                tasks.spawn(async move {
                    let tls_stream = match acceptor.accept(tcp_stream).await {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::debug!("TLS handshake failed from {remote_addr}: {e}");
                            return;
                        }
                    };

                    handle_connection(tls_stream, remote_addr, builder, app).await;
                });
            }
            () = &mut shutdown => {
                tracing::info!("Stopping TLS listener...");
                break;
            }
        }
    }

    // Await all in-flight connections with a timeout.
    let conn_timeout = std::time::Duration::from_secs(5);
    if tasks.is_empty() {
        tracing::info!("Server shut down gracefully.");
    } else {
        let start = std::time::Instant::now();
        loop {
            if tasks.is_empty() {
                tracing::info!("Server shut down gracefully.");
                break;
            }
            if start.elapsed() >= conn_timeout {
                tracing::warn!(
                    "Shutdown timed out after {:?}, {} connection(s) forcibly closed",
                    conn_timeout,
                    tasks.len()
                );
                break;
            }
            let remaining = conn_timeout.saturating_sub(start.elapsed());
            if tokio::time::timeout(remaining, tasks.join_next())
                .await
                .is_err()
            {
                if tasks.is_empty() {
                    tracing::info!("Server shut down gracefully.");
                } else {
                    tracing::warn!(
                        "Shutdown timed out after {:?}, {} connection(s) forcibly closed",
                        conn_timeout,
                        tasks.len()
                    );
                }
                break;
            }
        }
    }

    // Signal revocation cleanup to stop.
    if let Some((tx, _)) = rev_cleanup {
        let _ = tx.send(());
    }
}

/// Handle a single accepted connection by wrapping it in hyper IO and serving
/// HTTP requests through the axum router.
///
/// This is generic over the IO stream type so it works for both plain TCP
/// and TLS-wrapped connections.
async fn handle_connection<I>(
    io_stream: I,
    remote_addr: std::net::SocketAddr,
    builder: hyper_util::server::conn::auto::Builder<hyper_util::rt::tokio::TokioExecutor>,
    app: axum::Router,
) where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    use hyper::body::Incoming;
    use hyper_util::rt::tokio::TokioIo;
    use tower::Service;

    let io = TokioIo::new(io_stream);
    let service = hyper::service::service_fn(move |mut req: hyper::Request<Incoming>| {
        req.extensions_mut().insert(remote_addr);
        let mut app = app.clone();
        async move { app.call(req).await }
    });

    if let Err(e) = builder.serve_connection(io, service).await {
        tracing::debug!("Connection error from {remote_addr}: {e}");
    }
}

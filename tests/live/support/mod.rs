//! Support infrastructure for live tests.
//!
//! Provides [`BinaryHandle`] to spawn and manage the main-serve binary,
//! [`LiveClient`] for making HTTP requests against a running server, and
//! config-writing helpers to create temporary YAML configs per test.

use std::env;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_yaml;
use tempfile::{TempDir, NamedTempFile};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::timeout;

mod http;
pub use http::LiveClient;

/// Default timeout for the server to become ready after spawning.
const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(10);

/// Default timeout for graceful shutdown.
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Config scaffolding
// ---------------------------------------------------------------------------

/// Write a YAML config string to a temp directory and return the path.
///
/// The `TempDir` is returned alongside the path so the caller controls its
/// lifetime. The config file is always named `main-serve.yaml`.
pub fn write_config(temp_dir: &TempDir, yaml: &str) -> PathBuf {
    let config_path = temp_dir.path().join("main-serve.yaml");
    let mut file = NamedTempFile::new_in(temp_dir.path())
        .expect("failed to create temp config file");
    file.write_all(yaml.as_bytes())
        .expect("failed to write config");
    match file.persist(&config_path) {
            Ok(_) => {},
            Err(_) => {
                // Fallback: write directly if persist fails (e.g., cross-device).
                std::fs::write(&config_path, yaml).expect("failed to write config");
            }
        }
    // We need the path, not the file handle.
    // If persist failed, the file is still at config_path from the fallback.
    config_path
}

/// Write a config from a Rust struct (via serde) to a temp directory.
pub fn write_config_from_struct<T: serde::Serialize>(
    temp_dir: &TempDir,
    config: &T,
) -> PathBuf {
    let yaml = serde_yaml::to_string(config).expect("failed to serialize config to YAML");
    write_config(temp_dir, &yaml)
}

/// Write arbitrary files to a temp directory (useful for SPA files, assets, etc.).
pub fn write_files_to_dir(dir: &Path, files: &[(&str, &[u8])]) {
    for (relative_path, content) in files {
        let file_path = dir.join(relative_path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).expect("failed to create directory");
        }
        std::fs::write(&file_path, content).expect("failed to write file");
    }
}

// ---------------------------------------------------------------------------
// BinaryHandle
// ---------------------------------------------------------------------------

/// Handle to a running `main-serve` process.
///
/// Spawns the binary with the given config, waits for it to be ready on a
/// random port (parsed from stdout/stderr), and provides a [`LiveClient`]
/// for making HTTP requests.
///
/// On `Drop`, sends SIGTERM and waits up to [`DEFAULT_SHUTDOWN_TIMEOUT`] for
/// graceful shutdown. If the process doesn't exit, it's killed.
pub struct BinaryHandle {
    config_path: PathBuf,
    temp_dir: TempDir,
    process: tokio::process::Child,
    base_url: String,
    _assets: TempDir, // keeps asset directories alive
}

impl BinaryHandle {
    /// Spawn `main-serve` with the given YAML config.
    ///
    /// The binary is looked up from the `MAIN_SERVE_BIN` environment variable,
    /// or falls back to `$CARGO_BIN_EXE_main-serve`.
    ///
    /// This function waits up to `ready_timeout` for the server to start
    /// listening. It parses the port from the log line:
    /// `Main Serve listening on http://127.0.0.1:PORT`
    pub async fn spawn(
        yaml: &str,
        ready_timeout: Option<Duration>,
    ) -> Result<Self, SpawnError> {
        let temp_dir = TempDir::new().map_err(SpawnError::TempDir)?;

        let config_path = write_config(&temp_dir, yaml);

        let binary_path = env::var("MAIN_SERVE_BIN")
            .or_else(|_| env::var("CARGO_BIN_EXE_main-serve"))
            .map_err(|_| SpawnError::BinaryNotFound)?;

        let mut command = Command::new(&binary_path);
        command
            .arg("-c")
            .arg(&config_path)
            .env("MAIN_SERVE_ADMIN_TOKEN", "test-admin-token")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let process = command
            .spawn()
            .map_err(|e| SpawnError::ProcessSpawn(PathBuf::from(&binary_path), e))?;

        let ready_timeout = ready_timeout.unwrap_or(DEFAULT_READY_TIMEOUT);
        let (base_url, process) = Self::wait_for_ready(process, ready_timeout).await?;

        Ok(Self {
            config_path,
            temp_dir,
            process,
            base_url,
            _assets: TempDir::new().map_err(SpawnError::TempDir)?,
        })
    }

    /// Spawn `main-serve` with pre-written config path and extra args.
    pub async fn spawn_with_config(
        config_path: PathBuf,
        temp_dir: TempDir,
        extra_args: &[&str],
    ) -> Result<Self, SpawnError> {
        let binary_path = env::var("MAIN_SERVE_BIN")
            .or_else(|_| env::var("CARGO_BIN_EXE_main-serve"))
            .map_err(|_| SpawnError::BinaryNotFound)?;

        let mut command = Command::new(&binary_path);
        command
            .arg("-c")
            .arg(&config_path)
            .args(extra_args)
            .env("MAIN_SERVE_ADMIN_TOKEN", "test-admin-token")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let process = command
            .spawn()
            .map_err(|e| SpawnError::ProcessSpawn(PathBuf::from(&binary_path), e))?;

        let (base_url, process) = Self::wait_for_ready(process, DEFAULT_READY_TIMEOUT).await?;

        Ok(Self {
            config_path,
            temp_dir,
            process,
            base_url,
            _assets: TempDir::new().map_err(SpawnError::TempDir)?,
        })
    }

    /// Wait for the server to start listening.
    ///
    /// Reads from stdout/stderr looking for the "listening on" log line and
    /// extracts the port number.
    async fn wait_for_ready(
        mut process: tokio::process::Child,
        timeout_duration: Duration,
    ) -> Result<(String, tokio::process::Child), SpawnError> {
        let stdout = process
            .stdout
            .take()
            .ok_or_else(|| SpawnError::NoStdout)?;
        let stderr = process
            .stderr
            .take()
            .ok_or_else(|| SpawnError::NoStderr)?;

        let (tx, mut rx) = mpsc::unbounded_channel::<String>();

        // Read stdout into the channel
        let stdout_clone = tx.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let reader = tokio::io::BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = stdout_clone.send(line);
            }
        });

        // Read stderr into the channel (not shown in test output by default)
        let _stderr_clone = tx.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let reader = tokio::io::BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = _stderr_clone.send(format!("[stderr] {line}"));
            }
        });

        drop(tx); // drop the sender so the receiver knows when all readers are done

        let start = Instant::now();
        let mut last_line = String::new();

        loop {
            if start.elapsed() > timeout_duration {
                let _ = process.kill().await;
                return Err(SpawnError::Timeout {
                    timeout: timeout_duration,
                    last_output: last_line,
                });
            }

            match timeout(Duration::from_millis(100), rx.recv()).await {
                Ok(Some(line)) => {
                    last_line = line.clone();
                    if let Some(port) = extract_port(&line) {
                        let url = format!("http://127.0.0.1:{port}");
                        // Verify the port is actually reachable with a quick HTTP check
                        if Self::verify_ready(&url).await {
                            return Ok((url, process));
                        }
                    }
                }
                Ok(None) => {
                    // Channel closed, process may have exited
                    let status = process.try_wait().map_err(|e| SpawnError::WaitError(e))?;
                    if let Some(status) = status {
                        return Err(SpawnError::ProcessExited {
                            status,
                            last_output: last_line,
                        });
                    }
                    return Err(SpawnError::Timeout {
                        timeout: timeout_duration,
                        last_output: last_line,
                    });
                }
                Err(_) => continue,
            }
        }
    }

    /// Quick HTTP check to verify the server is actually ready to accept
    /// connections (not just bound the listener).
    async fn verify_ready(url: &str) -> bool {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()
            .ok();

        match client {
            Some(c) => {
                c.get(format!("{url}/")).send().await.is_ok()
            }
            None => false,
        }
    }

    /// The base URL the server is listening on.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Create a [`LiveClient`] configured for this server.
    pub fn client(&self) -> LiveClient {
        LiveClient::new(self.base_url())
    }

    /// Send SIGTERM for graceful shutdown.
    pub async fn shutdown(mut self) -> Result<(), SpawnError> {
        let start = Instant::now();
        let shutdown_timeout = DEFAULT_SHUTDOWN_TIMEOUT;

        // Send SIGTERM on Unix
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            if let Some(pid) = self.process.id() {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
        }

        // Wait for process to exit
        loop {
            if start.elapsed() > shutdown_timeout {
                self.process.kill().await.ok();
                let _ = self.process.wait().await;
                return Err(SpawnError::ShutdownTimeout {
                    timeout: shutdown_timeout,
                });
            }

            match self.process.try_wait() {
                Ok(Some(status)) => {
                    let _ = status;
                    return Ok(());
                }
                Ok(None) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => {
                    return Err(SpawnError::WaitError(e));
                }
            }
        }
    }

    /// Return the temp directory path (for inspection on test failure).
    pub fn temp_dir(&self) -> &TempDir {
        &self.temp_dir
    }
}

impl Drop for BinaryHandle {
    fn drop(&mut self) {
        // On drop, try to kill the process to avoid leaving it running
        // if the test panicked before calling shutdown().
        let _ = self.process.start_kill();
    }
}

/// Extract the port number from a log line like:
/// `Main Serve listening on http://127.0.0.1:12345`
/// or `Main Serve listening on https://127.0.0.1:12345`
/// Also handles JSON format: {"fields":{"message":"Main Serve listening on http://127.0.0.1:12345"}}
fn extract_port(line: &str) -> Option<u16> {
    // Try JSON format first
    if line.starts_with('{') {
        if let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(message) = obj.get("fields")?.get("message")?.as_str() {
                return extract_port(message);
            }
        }
    }

    // Look for the pattern "listening on protocol://host:port"
    let patterns = [
        "listening on http://",
        "listening on https://",
    ];

    for pattern in &patterns {
        if let Some(pos) = line.find(pattern) {
            let after = &line[pos + pattern.len()..];
            // Extract everything after the last '/' (handles host:port)
            if let Some(colon_pos) = after.rfind(':') {
                let port_str = &after[colon_pos + 1..];
                if let Ok(port) = port_str.trim().parse::<u16>() {
                    return Some(port);
                }
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur when spawning the binary.
#[derive(thiserror::Error, Debug)]
pub enum SpawnError {
    #[error("failed to create temp directory: {0}")]
    TempDir(#[from] std::io::Error),

    #[error("binary not found: set MAIN_SERVE_BIN or ensure CARGO_BIN_EXE_main-serve is set")]
    BinaryNotFound,

    #[error("failed to spawn binary '{0}': {1}")]
    ProcessSpawn(PathBuf, std::io::Error),

    #[error("no stdout available from process")]
    NoStdout,

    #[error("no stderr available from process")]
    NoStderr,

    #[error("server did not become ready within {:?}. Last output:\n{}", .timeout, .last_output)]
    Timeout { timeout: Duration, last_output: String },

    #[error("process exited before becoming ready: {status}. Last output:\n{last_output}")]
    ProcessExited { status: std::process::ExitStatus, last_output: String },

    #[error("failed to wait on process: {0}")]
    WaitError(std::io::Error),

    #[error("server did not shut down within {:?}, killing...", .timeout)]
    ShutdownTimeout { timeout: Duration },
}

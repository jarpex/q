use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use crate::config::{config_dir, CookieSet};

const PYTHON_SCRIPT: &str = include_str!("worker.py");

/// Configuration options for spawning the Gemini API background process.
pub struct AskOptions<'a> {
    /// The path to the Python executable.
    pub python_bin: &'a Path,
    /// The authenticated session cookies.
    pub cookies: &'a CookieSet,
    /// The user's query or prompt.
    pub query: &'a str,
    /// The specific AI model to use.
    pub model: &'a str,
    /// Whether to stream the response or process it in batch.
    pub stream: bool,
    /// Whether to enable debug logging.
    pub debug: bool,
}

/// Events emitted by the Gemini API background process.
pub enum GeminiEvent {
    /// A chunk of text received from the API.
    Chunk(String),
    /// An error occurred during the API interaction.
    Error(String),
    /// The API stream has finished successfully.
    Done,
}

/// Lists the models available for the authenticated account.
///
/// # Errors
///
/// Returns an error if the Python worker cannot be started, fails, or returns
/// invalid model data.
pub async fn list_models(
    python_bin: &Path,
    cookies: &CookieSet,
    debug: bool,
) -> Result<Vec<String>> {
    let cookie_cache_dir = config_dir()?.join("gemini-cache");
    let output = Command::new(python_bin)
        .arg("-c")
        .arg(PYTHON_SCRIPT)
        .arg(cookies.psid.as_str())
        .arg(cookies.psidts.as_str())
        .arg("")
        .arg("list-models")
        .env("GEMINI_COOKIE_PATH", cookie_cache_dir)
        .env("Q_GEMINI_VERBOSE", if debug { "1" } else { "0" })
        .output()
        .await
        .with_context(|| format!("Failed to spawn {}", python_bin.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr
            .lines()
            .find(|line| !line.is_empty())
            .unwrap_or("unknown error");
        anyhow::bail!("Python gemini_webapi failed: {detail}");
    }

    serde_json::from_slice(&output.stdout).context("Invalid model list returned by Python")
}

/// Spawns a background Python process to handle Gemini API interactions.
///
/// # Errors
///
/// This function will return an error if:
/// - The Python binary fails to spawn.
/// - The system fails to write the query to the process's standard input.
/// - Standard output or error streams cannot be extracted from the child process.
pub async fn spawn_gemini_stream(options: AskOptions<'_>) -> Result<mpsc::Receiver<GeminiEvent>> {
    let AskOptions {
        python_bin,
        cookies,
        query,
        model,
        stream,
        debug,
    } = options;

    if debug {
        // Allowed explicitly to satisfy `clippy::print_stderr` for debug logs
        #[allow(clippy::print_stderr)]
        {
            eprintln!("[debug] Python: {}", python_bin.display());
            eprintln!("[debug] Model: {model}");
            eprintln!("[debug] Stream: {stream}");
        }
    }

    let mode = if stream { "stream" } else { "batch" };
    let cookie_cache_dir = config_dir()?.join("gemini-cache");

    let mut child = Command::new(python_bin)
        .arg("-c")
        .arg(PYTHON_SCRIPT)
        .arg(cookies.psid.as_str())
        .arg(cookies.psidts.as_str())
        .arg(model)
        .arg(mode)
        .env("GEMINI_COOKIE_PATH", cookie_cache_dir)
        .env("Q_GEMINI_VERBOSE", if debug { "1" } else { "0" })
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to spawn {}", python_bin.display()))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(query.as_bytes())
            .await
            .context("Failed to write query to Python stdin")?;
    }

    let stdout = child.stdout.take().context("no stdout from python")?;
    let stderr = child.stderr.take().context("no stderr from python")?;

    let (tx, rx) = mpsc::channel(32);
    let (stdout_done_tx, stdout_done_rx) = oneshot::channel();

    let stdout_tx = tx.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut buf = vec![0u8; 8192];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => {
                    break;
                }
                Ok(n) => {
                    // Use .get() instead of direct slicing to satisfy `clippy::indexing_slicing`
                    let text = String::from_utf8_lossy(buf.get(..n).unwrap_or(&[])).to_string();
                    if stdout_tx.send(GeminiEvent::Chunk(text)).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = stdout_tx.send(GeminiEvent::Error(e.to_string())).await;
                    break;
                }
            }
        }

        let _ = stdout_done_tx.send(());
    });

    let stderr_tx = tx;
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf).await;

        let wait_result = child.wait().await;
        let _ = stdout_done_rx.await;
        if let Ok(status) = wait_result {
            if status.success() {
                let _ = stderr_tx.send(GeminiEvent::Done).await;
            } else {
                let stderr = String::from_utf8_lossy(&buf);
                let first_line = stderr
                    .lines()
                    .find(|l| !l.is_empty())
                    .unwrap_or("unknown error");
                let error_detail = if debug { stderr.trim() } else { first_line };
                let _ = stderr_tx
                    .send(GeminiEvent::Error(format!(
                        "Python gemini_webapi failed: {error_detail}"
                    )))
                    .await;
            }
        } else {
            let _ = stderr_tx
                .send(GeminiEvent::Error(
                    "Failed to wait for Python process".to_owned(),
                ))
                .await;
        }
    });

    Ok(rx)
}

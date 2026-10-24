use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use crate::config::{config_dir, CookieSet};

const PYTHON_SCRIPT: &str = include_str!("worker.py");

fn error_detail(stderr: &str, debug: bool) -> &str {
    if debug {
        return stderr.trim();
    }

    stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("Traceback"))
        .filter(|line| !line.starts_with("File "))
        .filter(|line| !is_log_line(line))
        .rfind(|line| is_error_line(line))
        .or_else(|| {
            stderr
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty() && !is_log_line(line))
        })
        .unwrap_or("unknown error")
}

fn is_log_line(line: &str) -> bool {
    [
        " | DEBUG ",
        " | INFO ",
        " | SUCCESS ",
        " | WARNING ",
        " | ERROR ",
    ]
    .iter()
    .any(|marker| line.contains(marker))
}

fn is_error_line(line: &str) -> bool {
    line.starts_with("Error:")
        || line.split_once(':').is_some_and(|(prefix, _)| {
            !prefix.is_empty()
                && prefix
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
                && prefix
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_uppercase())
        })
}

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

#[derive(Debug, Deserialize)]
/// Usage metrics and model quotas returned by Gemini.
pub struct UsageLimits {
    /// Compute-based usage windows and account tier information.
    pub usage_info: Value,
    /// Model-specific quota buckets and reset information.
    pub quotas: Value,
}

/// Verifies that the Gemini service accepts the given session cookies.
///
/// This runs the worker in its `verify` mode, which performs a real request
/// to Gemini with the cookies. A session that only looks right but does not
/// work is reported as a failure here, instead of failing later on the first
/// query.
///
/// # Errors
///
/// Returns an error if the Python worker cannot be started or fails, which
/// means that Gemini did not accept the cookies.
pub fn verify_session(python_bin: &Path, cookies: &CookieSet) -> Result<()> {
    let cookie_cache_dir = config_dir()?.join("gemini-cache");
    let output = std::process::Command::new(python_bin)
        .arg("-c")
        .arg(PYTHON_SCRIPT)
        .arg("verify")
        .env("GEMINI_COOKIE_PATH", cookie_cache_dir)
        .env("Q_GEMINI_VERBOSE", "0")
        .env("GEMINI_PSID", cookies.psid.as_str())
        .env("GEMINI_PSIDTS", cookies.psidts.as_str())
        .output()
        .with_context(|| format!("Failed to spawn {}", python_bin.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = error_detail(&stderr, false);
        anyhow::bail!("Gemini did not accept the session: {detail}");
    }

    Ok(())
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
        .arg("list-models")
        .env("GEMINI_COOKIE_PATH", cookie_cache_dir)
        .env("Q_GEMINI_VERBOSE", if debug { "1" } else { "0" })
        .env("GEMINI_PSID", cookies.psid.as_str())
        .env("GEMINI_PSIDTS", cookies.psidts.as_str())
        .output()
        .await
        .with_context(|| format!("Failed to spawn {}", python_bin.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = error_detail(&stderr, debug);
        anyhow::bail!("Python gemini_webapi failed: {detail}");
    }

    serde_json::from_slice(&output.stdout).context("Invalid model list returned by Python")
}

/// Retrieves usage limits and model quotas for the authenticated account.
///
/// # Errors
///
/// Returns an error if the Python worker cannot be started, fails, or returns
/// invalid usage data.
pub async fn usage_limits(
    python_bin: &Path,
    cookies: &CookieSet,
    debug: bool,
) -> Result<UsageLimits> {
    let cookie_cache_dir = config_dir()?.join("gemini-cache");
    let output = Command::new(python_bin)
        .arg("-c")
        .arg(PYTHON_SCRIPT)
        .arg("usage-limits")
        .env("GEMINI_COOKIE_PATH", cookie_cache_dir)
        .env("Q_GEMINI_VERBOSE", if debug { "1" } else { "0" })
        .env("GEMINI_PSID", cookies.psid.as_str())
        .env("GEMINI_PSIDTS", cookies.psidts.as_str())
        .output()
        .await
        .with_context(|| format!("Failed to spawn {}", python_bin.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = error_detail(&stderr, debug);
        anyhow::bail!("Python gemini_webapi failed: {detail}");
    }

    serde_json::from_slice(&output.stdout).context("Invalid usage limits returned by Python")
}

/// Spawns a background Python process to handle Gemini API interactions.
///
/// # Errors
///
/// Returns an error if:
/// - The Python binary fails to spawn.
/// - The system fails to write the query to the process's standard input.
/// - Standard output or error streams cannot be extracted from the child process.
#[allow(clippy::too_many_lines)]
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
        .arg(mode)
        .arg(model)
        .env("GEMINI_COOKIE_PATH", cookie_cache_dir)
        .env("Q_GEMINI_VERBOSE", if debug { "1" } else { "0" })
        .env("GEMINI_PSID", cookies.psid.as_str())
        .env("GEMINI_PSIDTS", cookies.psidts.as_str())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
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
        let mut carry = Vec::new();
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(300);

        loop {
            if tokio::time::Instant::now() >= deadline {
                let _ = stdout_tx
                    .send(GeminiEvent::Error(
                        "Python process timed out and was terminated".to_owned(),
                    ))
                    .await;
                break;
            }
            match tokio::time::timeout(std::time::Duration::from_secs(30), reader.read(&mut buf))
                .await
            {
                Ok(Ok(0)) => {
                    if !carry.is_empty() {
                        let text = String::from_utf8_lossy(&carry).to_string();
                        let _ = stdout_tx.send(GeminiEvent::Chunk(text)).await;
                    }
                    break;
                }
                Ok(Ok(n)) => {
                    carry.extend_from_slice(buf.get(..n).unwrap_or(&[]));

                    match std::str::from_utf8(&carry) {
                        Ok(s) => {
                            if !s.is_empty()
                                && stdout_tx
                                    .send(GeminiEvent::Chunk(s.to_owned()))
                                    .await
                                    .is_err()
                            {
                                break;
                            }
                            carry.clear();
                        }
                        Err(e) => {
                            let valid_up_to = e.valid_up_to();
                            if valid_up_to > 0 {
                                if let Some(slice) = carry.get(..valid_up_to) {
                                    if let Ok(valid) = std::str::from_utf8(slice) {
                                        if stdout_tx
                                            .send(GeminiEvent::Chunk(valid.to_owned()))
                                            .await
                                            .is_err()
                                        {
                                            break;
                                        }
                                    }
                                }
                            }
                            carry.drain(..valid_up_to);
                        }
                    }
                }
                Ok(Err(e)) => {
                    let _ = stdout_tx.send(GeminiEvent::Error(e.to_string())).await;
                    break;
                }
                Err(_) => {
                    let _ = stdout_tx
                        .send(GeminiEvent::Error(
                            "Python process timed out reading stdout".to_owned(),
                        ))
                        .await;
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

        // Wait for the stdout task to finish first. The child can block writing to
        // a full stdout pipe, so `child.wait()` would wait forever.
        let _ = stdout_done_rx.await;

        match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
            Ok(Ok(status)) => {
                if status.success() {
                    let _ = stderr_tx.send(GeminiEvent::Done).await;
                } else {
                    let stderr = String::from_utf8_lossy(&buf);
                    let _ = stderr_tx
                        .send(GeminiEvent::Error(format!(
                            "Python gemini_webapi failed: {}",
                            error_detail(&stderr, debug)
                        )))
                        .await;
                }
            }
            Ok(Err(e)) => {
                let _ = stderr_tx
                    .send(GeminiEvent::Error(format!(
                        "Failed to wait for Python process: {e}"
                    )))
                    .await;
            }
            Err(_) => {
                // Timeout: kill the child explicitly. kill_on_drop(true) only
                // fires when the child is dropped, which has not happened yet.
                let _ = child.kill().await;
                let _ = stderr_tx
                    .send(GeminiEvent::Error(
                        "Python process timed out and was terminated".to_owned(),
                    ))
                    .await;
            }
        }
    });

    Ok(rx)
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::error_detail;

    #[test]
    fn extracts_exception_after_library_logs() {
        let stderr = "\
2026-10-14 08:54:02.360 | INFO | client:init - initialized
Traceback (most recent call last):
  File \"<string>\", line 1, in main
ValueError: Unknown model name: 'XXX'. Available registered models: gemini-auto
";

        assert_eq!(
            error_detail(stderr, false),
            "ValueError: Unknown model name: 'XXX'. Available registered models: gemini-auto"
        );
    }

    #[test]
    fn extracts_explicit_error_without_traceback() {
        assert_eq!(
            error_detail("DEBUG noise\nError: empty query received\n", false),
            "Error: empty query received"
        );
    }

    #[test]
    fn keeps_the_verdict_of_the_server() {
        let stderr = "\
2026-10-21 17:46:15.387 | WARNING  | gemini_webapi.client:_fetch_user_status:559 - Account status: UNAUTHENTICATED
2026-10-21 17:46:15.770 | SUCCESS  | gemini_webapi.client:init:346 - Gemini client initialized successfully.
Gemini reports the account as UNAUTHENTICATED: Session is not authenticated or cookies have expired.
";

        assert_eq!(
            error_detail(stderr, false),
            "Gemini reports the account as UNAUTHENTICATED: Session is not authenticated or cookies have expired."
        );
    }

    #[test]
    fn keeps_full_stderr_in_debug_mode() {
        let stderr = "INFO noise\nTraceback...\nValueError: bad model";
        assert_eq!(error_detail(stderr, true), stderr);
    }

    #[test]
    fn falls_back_to_non_log_output() {
        assert_eq!(
            error_detail("2026 | INFO | noisy log\nunexpected failure\n", false),
            "unexpected failure"
        );
    }

    #[test]
    fn parses_usage_limits_payload() {
        let payload = r#"{
            "usage_info": {"tier": {"label": "FREE"}, "current_5h": null},
            "quotas": {"flash": {"remaining": 455, "total": 600}}
        }"#;

        let parsed: Result<super::UsageLimits, _> = serde_json::from_str(payload);
        assert!(parsed.is_ok());
        let Ok(limits) = parsed else { return };

        assert!(limits
            .usage_info
            .get("tier")
            .and_then(|tier| tier.get("label"))
            .is_some_and(|label| label == "FREE"));
        assert!(limits
            .quotas
            .get("flash")
            .and_then(|flash| flash.get("remaining"))
            .is_some_and(|remaining| remaining == 455));
    }
}

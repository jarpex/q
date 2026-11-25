use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::config::{config_dir, CookieSet};
use crate::{Error, Result};

const PYTHON_SCRIPT: &str = include_str!("worker.py");

/// Serializes the extra Google cookies for the worker, as a JSON array of
/// `{name, value, domain}` objects. The worker rebuilds a domain-aware cookie
/// jar from it, so a host-only cookie of `gemini.google.com` is not sent to
/// `lh3.google.com`.
fn extra_cookies_json(cookies: &CookieSet) -> String {
    let items: Vec<serde_json::Value> = cookies
        .extra
        .iter()
        .map(|c| {
            serde_json::json!({
                "name": c.name.as_str(),
                "value": c.value.as_str(),
                "domain": c.domain,
            })
        })
        .collect();
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_owned())
}

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
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

    let cookie_cache_dir = config_dir()?.join("gemini-cache");
    let mut child = std::process::Command::new(python_bin)
        .arg("-c")
        .arg(PYTHON_SCRIPT)
        .arg("verify")
        .env("GEMINI_COOKIE_PATH", cookie_cache_dir)
        .env("Q_GEMINI_VERBOSE", "0")
        .env("GEMINI_PSID", cookies.psid.as_str())
        .env("GEMINI_PSIDTS", cookies.psidts.as_str())
        .env("GEMINI_COOKIES", extra_cookies_json(cookies))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::System(format!("Failed to spawn {}: {e}", python_bin.display())))?;

    let mut stderr_pipe = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stderr_pipe.take() {
            use std::io::Read;
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });

    let start = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(Error::Network(format!(
                "Session verification timed out after {}s",
                TIMEOUT.as_secs()
            )));
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    let stderr_bytes = reader.join().unwrap_or_default();
    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr_bytes);
        let detail = error_detail(&stderr, false);
        return Err(Error::Auth(format!(
            "Gemini did not accept the session: {detail}"
        )));
    }

    Ok(())
}

/// The most a one-shot call of the Python worker may take.
///
/// The worker can hang on a stuck network call, so a deadline turns the hang
/// into an error instead of freezing the caller.
const WORKER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// Runs the Python worker in a one-shot mode and returns what it wrote to stdout.
///
/// The child is killed when the deadline passes, so a worker that hangs on a
/// stuck request cannot freeze the caller.
///
/// # Errors
///
/// Returns an error if:
/// - The Python worker cannot be started.
/// - The worker does not answer within the deadline.
/// - The worker exits with a failure.
async fn run_worker(
    python_bin: &Path,
    mode: &str,
    cookies: &CookieSet,
    debug: bool,
) -> Result<Vec<u8>> {
    let cookie_cache_dir = config_dir()?.join("gemini-cache");
    let child = Command::new(python_bin)
        .arg("-c")
        .arg(PYTHON_SCRIPT)
        .arg(mode)
        .env("GEMINI_COOKIE_PATH", cookie_cache_dir)
        .env("Q_GEMINI_VERBOSE", if debug { "1" } else { "0" })
        .env("GEMINI_PSID", cookies.psid.as_str())
        .env("GEMINI_PSIDTS", cookies.psidts.as_str())
        .env("GEMINI_COOKIES", extra_cookies_json(cookies))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::System(format!("Failed to spawn {}: {e}", python_bin.display())))?;

    // Dropping the future on timeout drops the child, and `kill_on_drop` stops
    // it, so a hung worker is reaped instead of leaking.
    let output = tokio::time::timeout(WORKER_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| {
            Error::Gemini(format!(
                "The Python worker did not answer in {}s and was stopped",
                WORKER_TIMEOUT.as_secs()
            ))
        })?
        .map_err(|e| Error::System(format!("Failed to wait for the Python worker: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = error_detail(&stderr, debug);
        return Err(Error::Gemini(format!(
            "Python gemini_webapi failed: {detail}"
        )));
    }

    Ok(output.stdout)
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
    let stdout = run_worker(python_bin, "list-models", cookies, debug).await?;

    serde_json::from_slice(&stdout)
        .map_err(|e| Error::Gemini(format!("Invalid model list returned by Python: {e}")))
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
    let stdout = run_worker(python_bin, "usage-limits", cookies, debug).await?;

    serde_json::from_slice(&stdout)
        .map_err(|e| Error::Gemini(format!("Invalid usage limits returned by Python: {e}")))
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
        .env("GEMINI_COOKIES", extra_cookies_json(cookies))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::System(format!("Failed to spawn {}: {e}", python_bin.display())))?;

    let stdin = child.stdin.take();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::System("no stdout from python".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::System("no stderr from python".into()))?;

    let (tx, rx) = mpsc::channel(32);

    let stdin_task = stdin.map(|mut pipe| {
        let query_bytes = query.as_bytes().to_vec();
        tokio::spawn(async move {
            let _ = pipe.write_all(&query_bytes).await;
            let _ = pipe.flush().await;
        })
    });

    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf).await;
        buf
    });

    tokio::spawn(async move {
        const CHUNK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
        const MAX_CARRY_BYTES: usize = 64 * 1024;
        let mut reader = BufReader::new(stdout);
        let mut buf = vec![0u8; 8192];
        let mut carry = Vec::new();
        let stream_deadline = tokio::time::sleep(tokio::time::Duration::from_secs(300));
        tokio::pin!(stream_deadline);

        loop {
            tokio::select! {
                () = &mut stream_deadline => {
                    let _ = child.kill().await;
                    if let Some(ref task) = stdin_task {
                        task.abort();
                    }
                    stderr_task.abort();
                    let _ = tx
                        .send(GeminiEvent::Error(
                            "Python process timed out and was terminated".to_owned(),
                        ))
                        .await;
                    return;
                }
                () = tx.closed() => {
                    let _ = child.kill().await;
                    if let Some(ref task) = stdin_task {
                        task.abort();
                    }
                    stderr_task.abort();
                    return;
                }
                read_res = tokio::time::timeout(CHUNK_TIMEOUT, reader.read(&mut buf)) => {
                    match read_res {
                        Ok(Ok(0)) => {
                            if !carry.is_empty() {
                                let text = String::from_utf8_lossy(&carry).to_string();
                                let _ = tx.send(GeminiEvent::Chunk(text)).await;
                            }
                            break;
                        }
                        Ok(Ok(n)) => {
                            carry.extend_from_slice(buf.get(..n).unwrap_or(&[]));

                            // Bound the carry buffer so a stream of invalid UTF-8
                            // cannot exhaust memory.
                            if carry.len() > MAX_CARRY_BYTES {
                                let _ = child.kill().await;
                                if let Some(ref task) = stdin_task {
                                    task.abort();
                                }
                                stderr_task.abort();
                                let _ = tx
                                    .send(GeminiEvent::Error(
                                        "Python output exceeded the maximum carry buffer".to_owned(),
                                    ))
                                    .await;
                                return;
                            }

                            match std::str::from_utf8(&carry) {
                                Ok(s) => {
                                    if !s.is_empty()
                                        && tx
                                            .send(GeminiEvent::Chunk(s.to_owned()))
                                            .await
                                            .is_err()
                                    {
                                        let _ = child.kill().await;
                                        if let Some(ref task) = stdin_task {
                                            task.abort();
                                        }
                                        stderr_task.abort();
                                        return;
                                    }
                                    carry.clear();
                                }
                                Err(e) => {
                                    let valid_up_to = e.valid_up_to();
                                    if valid_up_to > 0 {
                                        if let Some(slice) = carry.get(..valid_up_to) {
                                            if let Ok(valid) = std::str::from_utf8(slice) {
                                                if tx
                                                    .send(GeminiEvent::Chunk(valid.to_owned()))
                                                    .await
                                                    .is_err()
                                                {
                                                    let _ = child.kill().await;
                                                    if let Some(ref task) = stdin_task {
                                                        task.abort();
                                                    }
                                                    stderr_task.abort();
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                    carry.drain(..valid_up_to);
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            let _ = child.kill().await;
                            if let Some(ref task) = stdin_task {
                                task.abort();
                            }
                            stderr_task.abort();
                            let _ = tx.send(GeminiEvent::Error(e.to_string())).await;
                            return;
                        }
                        Err(_) => {
                            let _ = child.kill().await;
                            if let Some(ref task) = stdin_task {
                                task.abort();
                            }
                            stderr_task.abort();
                            let _ = tx
                                .send(GeminiEvent::Error(
                                    "Python process timed out reading stdout".to_owned(),
                                ))
                                .await;
                            return;
                        }
                    }
                }
            }
        }

        if let Some(ref task) = stdin_task {
            task.abort();
        }

        match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
            Ok(Ok(status)) => {
                let stderr_buf = match tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    stderr_task,
                )
                .await
                {
                    Ok(Ok(buf)) => buf,
                    _ => Vec::new(),
                };
                if status.success() {
                    let _ = tx.send(GeminiEvent::Done).await;
                } else {
                    let stderr = String::from_utf8_lossy(&stderr_buf);
                    let _ = tx
                        .send(GeminiEvent::Error(format!(
                            "Python gemini_webapi failed: {}",
                            error_detail(&stderr, debug)
                        )))
                        .await;
                }
            }
            Ok(Err(e)) => {
                let _ = child.kill().await;
                stderr_task.abort();
                let _ = tx
                    .send(GeminiEvent::Error(format!(
                        "Failed to wait for Python process: {e}"
                    )))
                    .await;
            }
            Err(_) => {
                let _ = child.kill().await;
                stderr_task.abort();
                let _ = tx
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

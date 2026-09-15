use crate::config::{config_dir, CookieSet};
use anyhow::{Context, Result};
use std::io::Write as StdWrite;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const MIN_PYTHON: (u32, u32) = (3, 11);

fn parse_version(output: &str) -> Option<(u32, u32)> {
    let ver_token = output
        .split_whitespace()
        .find(|w| w.starts_with(|c: char| c.is_ascii_digit()))?;
    
    let mut parts = ver_token.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
    
    Some((major, minor))
}

fn try_python(path: &Path) -> bool {
    let Ok(output) = StdCommand::new(path).arg("--version").output() else {
        return false;
    };
    
    let text = String::from_utf8_lossy(&output.stdout);
    let text_err = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{text}{text_err}");

    parse_version(&combined).is_some_and(|ver| ver >= MIN_PYTHON)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file() && path.metadata().is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
        
        #[cfg(windows)]
        {
            for ext in ["exe", "cmd", "bat"] {
                let mut c = candidate.clone();
                c.set_extension(ext);
                if is_executable(&c) {
                    return Some(c);
                }
            }
        }
    }
    None
}

pub(crate) fn find_system_python() -> Result<PathBuf> {
    let mut candidates: Vec<String> = vec!["python3".into(), "python".into()];
    
    #[cfg(windows)]
    candidates.push("py".into());
    
    for minor in MIN_PYTHON.1..=15 {
        candidates.push(format!("python3.{minor}"));
    }

    for candidate in &candidates {
        if let Some(path) = find_in_path(candidate) {
            if try_python(&path) {
                return Ok(path);
            }
        }
        
        let direct_path = PathBuf::from(candidate);
        if try_python(&direct_path) {
            return Ok(direct_path);
        }
    }

    let min_major = MIN_PYTHON.0;
    let min_minor = MIN_PYTHON.1;
    anyhow::bail!(
        "No suitable Python {min_major}.{min_minor}+ found on your system.\n\
         Please install it or ensure it is in your PATH."
    )
}

fn venv_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("venv"))
}

fn venv_python_bin(venv_path: &Path) -> PathBuf {
    if cfg!(windows) {
        venv_path.join("Scripts").join("python.exe")
    } else {
        venv_path.join("bin").join("python")
    }
}

fn venv_is_healthy(python_bin: &Path) -> bool {
    if !python_bin.exists() {
        return false;
    }
    StdCommand::new(python_bin)
        .arg("-c")
        .arg("from gemini_webapi import GeminiClient; print('ok')")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[allow(clippy::print_stdout, clippy::print_stderr)]
pub(crate) fn ensure_python_venv(force_recreate: bool) -> Result<PathBuf> {
    let venv_path = venv_dir()?;
    let python_bin = venv_python_bin(&venv_path);

    if !force_recreate && venv_is_healthy(&python_bin) {
        return Ok(python_bin);
    }

    let system_python = find_system_python()?;

    if venv_path.exists() {
        let display_path = venv_path.display();
        println!("Removing stale venv at {display_path}...");
        if let Err(e) = std::fs::remove_dir_all(&venv_path) {
            eprintln!("Warning: Failed to remove stale venv: {e}");
        }
    }

    let display_venv = venv_path.display();
    let display_python = system_python.display();
    println!(
        "Creating Python virtual environment at {display_venv} using {display_python}..."
    );
    
    let create_status = StdCommand::new(&system_python)
        .args(["-m", "venv"])
        .arg(&venv_path)
        .status()
        .context("failed to run `python -m venv`")?;

    if !create_status.success() {
        anyhow::bail!("Failed to create virtual environment");
    }

    println!("Upgrading pip...");
    let pip_upgrade = StdCommand::new(&python_bin)
        .args(["-m", "pip", "install", "--upgrade", "pip"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok();
        
    if pip_upgrade.is_some_and(|s| !s.success()) {
        eprintln!("Warning: pip upgrade failed, continuing anyway");
    }

    println!("Installing gemini-webapi...");
    let install_status = StdCommand::new(&python_bin)
        .args(["-m", "pip", "install", "-U", "gemini-webapi", "httpx"])
        .status()
        .context("failed to run pip install")?;

    if !install_status.success() {
        let display_bin = python_bin.display();
        anyhow::bail!(
            "Failed to install gemini-webapi in venv. \
             Run `{display_bin} -m pip install -U gemini-webapi` to see details."
        );
    }

    if !venv_is_healthy(&python_bin) {
        anyhow::bail!("venv was created but gemini_webapi cannot be imported");
    }

    println!("Python venv ready at {display_venv}");
    Ok(python_bin)
}

pub(crate) const PYTHON_SCRIPT: &str = r#"
import asyncio
import sys
import traceback

try:
    from gemini_webapi import GeminiClient
except ImportError:
    print(
        f"Error: gemini-webapi is not installed for this Python ({sys.executable}).\n"
        f"Run the application with the venv rebuild flag to fix this.",
        file=sys.stderr,
    )
    sys.exit(2)


class Cleaner:
    """Strips service XML tags from a stream."""

    BLOCKS = {"ElicitationsGroup"}

    def __init__(self):
        self.state = "TEXT"
        self.buf = ""
        self.block_name = ""

    def feed(self, text):
        out = []
        for ch in text:
            if self.state == "TEXT":
                if ch == "<":
                    self.state = "TAG"
                    self.buf = "<"
                else:
                    out.append(ch)
            elif self.state == "TAG":
                self.buf += ch
                if ch == ">":
                    tag_content = self.buf[1:-1].strip()
                    if tag_content.startswith("/"):
                        self.state = "TEXT"
                    else:
                        parts = tag_content.split()
                        if not parts:
                            self.state = "TEXT"
                        else:
                            name = parts[0].rstrip("/")
                            if name in self.BLOCKS:
                                self.state = "BLOCK"
                                self.block_name = name
                            else:
                                self.state = "TEXT"
                    self.buf = ""
            elif self.state == "BLOCK":
                self.buf += ch
                if self.buf.endswith(f"</{self.block_name}>"):
                    self.state = "TEXT"
                    self.buf = ""
        return "".join(out)


async def main():
    if len(sys.argv) < 5:
        print("Usage: python -c '<script>' <psid> <psidts> <model> <mode>", file=sys.stderr)
        sys.exit(1)

    psid = sys.argv[1]
    psidts = sys.argv[2]
    model_arg = sys.argv[3]
    model = model_arg if model_arg and model_arg != "None" else None
    mode = sys.argv[4]

    query = sys.stdin.read()
    if not query:
        print("Error: empty query received", file=sys.stderr)
        sys.exit(1)

    try:
        client = GeminiClient(psid, psidts)
        await client.init(timeout=60, auto_close=False, close_delay=300, auto_refresh=True)

        kwargs = {"temporary": True}
        if model:
            kwargs["model"] = model

        cleaner = Cleaner()

        if mode == "stream" and hasattr(client, "generate_content_stream"):
            prev_text = ""
            async for chunk in client.generate_content_stream(query, **kwargs):
                delta = getattr(chunk, "text_delta", None)
                if delta is None:
                    current_text = getattr(chunk, "text", "") or ""
                    if current_text.startswith(prev_text):
                        delta = current_text[len(prev_text):]
                    else:
                        delta = current_text
                    prev_text = current_text
                
                if delta:
                    cleaned = cleaner.feed(delta)
                    if cleaned:
                        sys.stdout.write(cleaned)
                        sys.stdout.flush()
            sys.stdout.write("\n")
            sys.stdout.flush()
        else:
            response = await client.generate_content(query, **kwargs)
            text = response.text or ""
            sys.stdout.write(cleaner.feed(text))
            sys.stdout.write("\n")
            sys.stdout.flush()
    except Exception:
        traceback.print_exc(file=sys.stderr)
        sys.exit(1)


asyncio.run(main())
"#;

#[allow(clippy::print_stderr)]
pub(crate) async fn ask_gemini_via_python(
    python_bin: &Path,
    cookies: &CookieSet,
    query: &str,
    model: &str,
    stream: bool,
    debug: bool,
) -> Result<String> {
    if debug {
        let display_bin = python_bin.display();
        eprintln!("[debug] Python: {display_bin}");
        eprintln!("[debug] Model: {model}");
        eprintln!("[debug] Stream: {stream}");
    }

    let mode = if stream { "stream" } else { "batch" };

    let mut child = Command::new(python_bin)
        .arg("-c")
        .arg(PYTHON_SCRIPT)
        .arg(&cookies.psid)
        .arg(&cookies.psidts)
        .arg(model)
        .arg(mode)
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

    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let mut reader = BufReader::new(stderr);
        let _ = reader.read_to_end(&mut buf).await;
        buf
    });

    let collected = if stream {
        let mut reader = BufReader::new(stdout);
        let mut buf = [0u8; 8192];
        let mut term = std::io::stdout();
        loop {
            let n = reader.read(&mut buf).await.context("failed to read python stdout")?;
            if n == 0 {
                break;
            }
            if let Some(slice) = buf.get(..n) {
                term.write_all(slice)?;
            }
            term.flush()?;
        }
        String::new()
    } else {
        let mut reader = BufReader::new(stdout);
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).await?;
        String::from_utf8_lossy(&buf).trim().to_string()
    };

    let status = child.wait().await.context("failed to wait for python")?;
    let stderr_bytes = stderr_task.await.unwrap_or_default();

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr_bytes);
        if !stderr.trim().is_empty() {
            eprintln!("\n=== Python error ===");
            eprint!("{stderr}");
            eprintln!("====================\n");
        }
        let first_line = stderr.lines().find(|l| !l.is_empty()).unwrap_or("unknown error");
        anyhow::bail!("Python gemini_webapi failed: {first_line}");
    }

    Ok(collected)
}
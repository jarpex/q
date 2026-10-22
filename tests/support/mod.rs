//! Shared helpers for the integration tests.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A directory under the system temporary directory that is deleted on drop.
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub(crate) fn new(tag: &str) -> anyhow::Result<Self> {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("q-it-{tag}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path());
    }
}

/// The Python stub that stands in for the `gemini_webapi` package.
const STUB_SOURCE: &str = include_str!("fixtures/gemini_webapi.py");

/// The Python stub that stands in for the `uc` program.
const UC_STUB_SOURCE: &str = include_str!("fixtures/fake_uc.py");

/// A q environment with a Python venv that answers like Gemini.
pub(crate) struct TestEnv {
    dir: TempDir,
    venv_python: PathBuf,
}

impl TestEnv {
    /// Prepares the environment. Returns `None` when no usable Python exists.
    pub(crate) fn new() -> anyhow::Result<Option<Self>> {
        let Some(python) = find_python() else {
            return Ok(None);
        };
        let dir = TempDir::new("env")?;
        let mut env = Self {
            dir,
            venv_python: PathBuf::new(),
        };
        env.build_venv(&python)?;
        env.venv_python = venv_python(&env.dir.path().join("venv"));
        env.write_metadata()?;
        env.write_cookies()?;
        Ok(Some(env))
    }

    fn build_venv(&self, python: &Path) -> anyhow::Result<()> {
        let venv = self.dir.path().join("venv");
        let status = Command::new(python)
            .args(["-m", "venv", "--without-pip"])
            .arg(&venv)
            .status()?;
        if !status.success() {
            return Err(anyhow::anyhow!("the venv module failed"));
        }

        let venv_python = venv_python(&venv);
        let output = Command::new(&venv_python)
            .args(["-c", "import site; print(site.getsitepackages()[0])"])
            .output()?;
        let site_packages = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if site_packages.is_empty() {
            return Err(anyhow::anyhow!("cannot find site-packages"));
        }

        let stub = PathBuf::from(site_packages).join("gemini_webapi.py");
        std::fs::write(&stub, STUB_SOURCE)?;
        Ok(())
    }

    fn write_metadata(&self) -> anyhow::Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let metadata = format!(
            "{{\"last_update\": {now}, \"last_version\": \"{}\"}}",
            env!("CARGO_PKG_VERSION")
        );
        self.write_file("metadata.json", metadata.as_bytes())
    }

    fn write_cookies(&self) -> anyhow::Result<()> {
        self.write_file(
            "cookies.json",
            b"{\"psid\":\"test-psid\",\"psidts\":\"test-psidts\"}",
        )?;
        set_private_permissions(&self.dir.path().join("cookies.json"));
        Ok(())
    }

    fn write_file(&self, name: &str, content: &[u8]) -> anyhow::Result<()> {
        Ok(std::fs::write(self.dir.path().join(name), content)?)
    }

    /// Returns the configuration directory of the environment.
    pub(crate) fn config_dir(&self) -> &Path {
        self.dir.path()
    }

    /// Sets the text that the stub answers with.
    pub(crate) fn set_response(&self, text: &str) -> anyhow::Result<()> {
        self.write_file("stub_response.txt", text.as_bytes())
    }

    /// Writes a configuration file into the environment.
    pub(crate) fn write_config(&self, content: &str) -> anyhow::Result<()> {
        self.write_file("q.conf", content.as_bytes())
    }

    /// Writes a stand-in for the uc program and returns its path.
    ///
    /// The program is a Python script, so it runs the same way on every
    /// platform the tests run on.
    pub(crate) fn write_uc_stub(&self) -> anyhow::Result<PathBuf> {
        let script = self.dir.path().join("uc_stub.py");
        std::fs::write(&script, UC_STUB_SOURCE)?;

        let launcher = self.dir.path().join(if cfg!(windows) {
            "uc_stub.cmd"
        } else {
            "uc_stub.sh"
        });
        let body = if cfg!(windows) {
            format!(
                "@echo off\r\n\"{}\" \"{}\" %*\r\n",
                self.venv_python.display(),
                script.display()
            )
        } else {
            format!(
                "#!/bin/sh\nexec \"{}\" \"{}\" \"$@\"\n",
                self.venv_python.display(),
                script.display()
            )
        };
        std::fs::write(&launcher, body)?;
        make_executable(&launcher);

        Ok(launcher)
    }

    /// Returns the arguments that the stand-in for the uc program was called with.
    pub(crate) fn uc_args(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("uc_args.txt")).unwrap_or_default()
    }

    /// Returns the requests that the stub received.
    pub(crate) fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("stub_log.jsonl")).unwrap_or_default()
    }

    /// Stores the token of a platform, the way the screen of the tokens does.
    pub(crate) fn set_key(&self, provider: &str, token: &str) -> anyhow::Result<()> {
        let path = self.dir.path().join("keys.json");
        let keys = serde_json::json!([{
            "provider": provider,
            "token": token,
        }]);
        std::fs::write(&path, keys.to_string())?;
        set_private_permissions(&path);
        Ok(())
    }

    /// Runs the q binary against this environment.
    pub(crate) fn run(&self, args: &[&str]) -> anyhow::Result<Output> {
        Ok(Command::new(env!("CARGO_BIN_EXE_q"))
            .args(args)
            .env("Q_CONFIG_DIR", self.dir.path())
            .env("Q_DISABLE_CLIPBOARD", "1")
            .output()?)
    }

    /// Runs the q binary and feeds it the given text on stdin.
    pub(crate) fn run_with_stdin(&self, args: &[&str], input: &str) -> anyhow::Result<Output> {
        use std::io::Write;
        use std::process::Stdio;

        let mut child = Command::new(env!("CARGO_BIN_EXE_q"))
            .args(args)
            .env("Q_CONFIG_DIR", self.dir.path())
            .env("Q_DISABLE_CLIPBOARD", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input.as_bytes());
        }
        Ok(child.wait_with_output()?)
    }
}

/// Returns the path of the Python interpreter inside a virtual environment.
fn venv_python(venv: &Path) -> PathBuf {
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    }
}

/// Returns a Python interpreter that can build a virtual environment.
fn find_python() -> Option<PathBuf> {
    for name in ["python3", "python"] {
        let result = Command::new(name).args(["-c", "import venv"]).output();
        if let Ok(output) = result {
            if output.status.success() {
                return Some(PathBuf::from(name));
            }
        }
    }
    None
}

/// Restricts a file to the owner, which the cookie loader requires.
#[cfg_attr(not(unix), allow(clippy::missing_const_for_fn))]
fn set_private_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Makes a file runnable by its owner.
#[cfg_attr(not(unix), allow(clippy::missing_const_for_fn))]
fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(not(unix))]
    let _ = path;
}

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::config_dir;

use super::finder::find_system_python;

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
    Command::new(python_bin)
        .arg("-c")
        .arg("from gemini_webapi import GeminiClient; print('ok')")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Ensures that a Python virtual environment exists and is healthy, creating or updating it as necessary.
///
/// # Errors
///
/// This function will return an error if:
/// - The application's configuration directory cannot be determined or accessed.
/// - A suitable system Python installation (meeting the minimum version requirement) cannot be found.
/// - The `python -m venv` command fails to create the virtual environment.
/// - The required Python dependencies (`gemini-webapi`, `httpx`) fail to install.
/// - The virtual environment is created but fails a health check (i.e., `gemini_webapi` cannot be imported).
#[allow(clippy::print_stdout, clippy::print_stderr)]
pub fn ensure_venv(force_recreate: bool, update_deps: bool) -> Result<PathBuf> {
    let venv_path = venv_dir()?;
    let python_bin = venv_python_bin(&venv_path);

    if !force_recreate && venv_is_healthy(&python_bin) {
        if update_deps {
            println!("Updating Python dependencies...");
            let update_status = Command::new(&python_bin)
                .args(["-m", "pip", "install", "-U", "gemini-webapi", "httpx"])
                .status()
                .context("failed to run pip install")?;

            if update_status.success() {
                println!("Dependencies updated successfully");
            } else {
                eprintln!("Warning: dependency update failed, continuing with existing versions");
            }
        }
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
    println!("Creating Python virtual environment at {display_venv} using {display_python}...");

    let create_status = Command::new(&system_python)
        .args(["-m", "venv"])
        .arg(&venv_path)
        .status()
        .context("failed to run `python -m venv`")?;

    if !create_status.success() {
        anyhow::bail!("Failed to create virtual environment");
    }

    println!("Upgrading pip...");
    let pip_upgrade = Command::new(&python_bin)
        .args(["-m", "pip", "install", "--upgrade", "pip"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok();

    if pip_upgrade.is_some_and(|s| !s.success()) {
        eprintln!("Warning: pip upgrade failed, continuing anyway");
    }

    println!("Installing gemini-webapi...");
    let install_status = Command::new(&python_bin)
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

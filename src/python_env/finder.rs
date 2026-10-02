use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

type Version = (u32, u32);

const MIN_PYTHON: Version = (3, 11);

fn parse_version(output: &str) -> Option<Version> {
    let ver_token = output
        .split_whitespace()
        .find(|w| w.starts_with(|c: char| c.is_ascii_digit()))?;

    let mut parts = ver_token.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    Some((major, minor))
}

fn try_python(path: &Path) -> bool {
    let Ok(output) = Command::new(path).arg("--version").output() else {
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
    path.is_file()
        && path
            .metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
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

/// Searches the system for a suitable Python executable that meets the minimum version requirement.
///
/// # Errors
///
/// Returns an error if no Python executable of the required minimum version (e.g., Python 3.11+)
/// can be found in the system's `PATH` or as a direct executable path.
pub fn find_system_python() -> Result<PathBuf> {
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

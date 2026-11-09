use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::path_util::find_in_path;

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

/// Searches the system for a suitable Python executable that meets the minimum version requirement.
///
/// # Errors
///
/// Returns an error if no Python of the minimum version (e.g., Python 3.11+) is in
/// the system `PATH` or given as a direct executable path.
pub fn find_system_python() -> Result<PathBuf> {
    let mut candidates: Vec<String> = vec!["python3".into(), "python".into()];

    #[cfg(windows)]
    candidates.push("py".into());

    for minor in MIN_PYTHON.1..=15 {
        candidates.push(format!("python3.{minor}"));
    }

    for candidate in &candidates {
        if let Some(path) = find_in_path(Path::new(candidate)) {
            if try_python(&path) {
                return Ok(path);
            }
        }

        #[cfg(not(windows))]
        {
            let direct_path = PathBuf::from(candidate);
            if try_python(&direct_path) {
                return Ok(direct_path);
            }
        }
    }

    let min_major = MIN_PYTHON.0;
    let min_minor = MIN_PYTHON.1;
    anyhow::bail!(
        "No suitable Python {min_major}.{min_minor}+ found on your system.\n\
         Please install it or ensure it is in your PATH."
    )
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::parse_version;

    #[test]
    fn parses_a_full_version() {
        assert_eq!(parse_version("Python 3.12.1"), Some((3, 12)));
    }

    #[test]
    fn parses_a_version_without_a_patch() {
        assert_eq!(parse_version("Python 3.11"), Some((3, 11)));
    }

    #[test]
    fn parses_a_bare_version() {
        assert_eq!(parse_version("3.9"), Some((3, 9)));
    }

    #[test]
    fn treats_a_missing_minor_as_zero() {
        assert_eq!(parse_version("Python 3"), Some((3, 0)));
    }

    #[test]
    fn skips_the_leading_text() {
        assert_eq!(parse_version("Python 2.7.18"), Some((2, 7)));
    }

    #[test]
    fn rejects_text_without_a_version() {
        assert_eq!(parse_version("no version here"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn rejects_a_non_numeric_version() {
        assert_eq!(parse_version("Python x.y"), None);
    }

    #[test]
    fn find_in_path_returns_none_for_missing_binary() {
        assert!(super::find_in_path(std::path::Path::new("non_existent_binary_12345")).is_none());
    }
}

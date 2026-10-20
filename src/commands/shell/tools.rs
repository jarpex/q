use std::env;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[cfg(not(unix))]
use std::ffi::OsString;

/// Checks if a given command-line tool exists on the system.
///
/// Returns `true` if the tool is found and the check passes, `false` otherwise.
struct ToolValidator {
    path_dirs: Vec<PathBuf>,
    #[cfg(not(unix))]
    pathext: Vec<String>,
}

impl ToolValidator {
    /// Creates a new validator, capturing the current PATH and PATHEXT.
    fn new() -> Self {
        let path_var = env::var_os("PATH").unwrap_or_default();
        let path_dirs: Vec<PathBuf> = env::split_paths(&path_var).collect();

        #[cfg(not(unix))]
        let pathext = parse_pathext();

        Self {
            path_dirs,
            #[cfg(not(unix))]
            pathext,
        }
    }

    /// Checks if a given command-line tool exists.
    fn validate(&self, tool: &str) -> bool {
        if tool.is_empty() {
            return false;
        }

        let path = Path::new(tool);

        let has_separator = path.components().count() > 1;

        if has_separator {
            #[cfg(not(unix))]
            return check_executable_windows(path, &self.pathext);
            #[cfg(unix)]
            return check_executable_unix(path);
        }

        for dir in &self.path_dirs {
            let candidate = dir.join(tool);

            #[cfg(not(unix))]
            if check_executable_windows(&candidate, &self.pathext) {
                return true;
            }

            #[cfg(unix)]
            if check_executable_unix(&candidate) {
                return true;
            }
        }

        false
    }
}

static VALIDATOR: OnceLock<ToolValidator> = OnceLock::new();

/// Checks if a given command-line tool exists on the system.
pub fn validate_tool(tool: &str) -> bool {
    let validator = VALIDATOR.get_or_init(ToolValidator::new);
    validator.validate(tool)
}

#[cfg(unix)]
fn check_executable_unix(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn parse_pathext() -> Vec<String> {
    let pathext = env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(".COM;.EXE;.BAT;.CMD"));
    let mut extensions: Vec<String> = pathext
        .to_string_lossy()
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.starts_with('.') {
                s.to_ascii_lowercase()
            } else {
                format!(".{}", s.to_ascii_lowercase())
            }
        })
        .collect();

    let defaults = [".exe", ".com", ".bat", ".cmd"];
    for ext in defaults {
        if !extensions.iter().any(|e| e == ext) {
            extensions.push(ext.to_owned());
        }
    }

    extensions
}

#[cfg(not(unix))]
fn has_executable_extension_windows(path: &Path, extensions: &[String]) -> bool {
    if let Some(ext) = path.extension() {
        let ext_str = ext.to_string_lossy();
        let ext_trimmed = ext_str.trim_start_matches('.');

        for valid_ext in extensions {
            let valid_trimmed = valid_ext.trim_start_matches('.');
            if ext_trimmed.eq_ignore_ascii_case(valid_trimmed) {
                return true;
            }
        }
    }
    false
}

#[cfg(not(unix))]
fn check_executable_windows(path: &Path, extensions: &[String]) -> bool {
    if path.is_file() && has_executable_extension_windows(path, extensions) {
        return true;
    }

    // Reuse the buffer for each candidate extension.
    if !has_executable_extension_windows(path, extensions) {
        let mut candidate = OsString::new();

        for ext in extensions {
            candidate.clear();
            candidate.push(path);
            candidate.push(ext);

            let candidate_path = Path::new(&candidate);
            if candidate_path.is_file() {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::validate_tool;

    #[test]
    fn rejects_an_empty_name() {
        assert!(!validate_tool(""));
    }

    #[test]
    fn rejects_a_name_that_does_not_exist() {
        assert!(!validate_tool("q-definitely-not-a-real-tool-xyz"));
    }

    #[test]
    fn rejects_a_path_that_does_not_exist() {
        assert!(!validate_tool("/nonexistent/q-tool"));
    }

    #[cfg(unix)]
    #[test]
    fn finds_a_tool_in_the_path() {
        // Every POSIX system has a shell.
        assert!(validate_tool("sh"));
    }

    #[cfg(unix)]
    #[test]
    fn finds_a_tool_by_its_path() {
        assert!(validate_tool("/bin/sh"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_file_that_is_not_executable() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let dir = crate::test_support::TempDir::new("tools")?;
        let path = dir.path().join("plain-file");
        std::fs::write(&path, b"data")?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;

        assert!(!validate_tool(&path.to_string_lossy()));
        Ok(())
    }
}

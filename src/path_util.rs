use std::path::{Path, PathBuf};

/// Checks if a file exists and is executable.
///
/// On Unix, checks that the file exists and at least one execute bit is set.
/// On Windows, checks that the file exists.
#[must_use]
pub fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }

    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Searches `PATH` for an executable by name.
///
/// Skips non-absolute directories on Windows to prevent binary planting from
/// the working directory. Tries standard executable extensions on Windows
/// when the name has no matching file.
#[must_use]
pub fn find_in_path(name: &Path) -> Option<PathBuf> {
    if name.as_os_str().is_empty() {
        return None;
    }

    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        #[cfg(windows)]
        if !dir.is_absolute() {
            continue;
        }

        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }

        #[cfg(windows)]
        {
            for ext in ["exe", "cmd", "bat"] {
                let mut with_ext = candidate.clone();
                with_ext.set_extension(ext);
                if is_executable(&with_ext) {
                    return Some(with_ext);
                }
            }
        }
    }
    None
}

/// Resolves a program for execution.
///
/// Returns paths with directory separators as given. Resolves bare names
/// against `PATH`.
#[must_use]
pub fn resolve_executable(program: &Path) -> Option<PathBuf> {
    if program.components().count() > 1 {
        return Some(program.to_path_buf());
    }
    find_in_path(program)
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{find_in_path, is_executable, resolve_executable};
    use std::path::Path;

    #[test]
    fn find_in_path_returns_none_for_missing_binary() {
        assert!(find_in_path(Path::new("non_existent_binary_12345")).is_none());
    }

    #[test]
    fn find_in_path_returns_none_for_empty_name() {
        assert!(find_in_path(Path::new("")).is_none());
    }

    #[test]
    fn resolve_executable_preserves_path_with_components() {
        let path = Path::new("sub/dir/binary");
        assert_eq!(resolve_executable(path), Some(path.to_path_buf()));
    }

    #[test]
    fn rejects_nonexistent_file() {
        assert!(!is_executable(Path::new("/nonexistent/file/12345")));
    }

    #[cfg(unix)]
    #[test]
    fn finds_system_binary_in_path() {
        assert!(find_in_path(Path::new("sh")).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_executable_file() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let dir = crate::test_support::TempDir::new("path_util")?;
        let file = dir.path().join("script.sh");
        std::fs::write(&file, b"echo hello")?;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644))?;

        assert!(!is_executable(&file));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn accepts_executable_file() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let dir = crate::test_support::TempDir::new("path_util_exec")?;
        let file = dir.path().join("script.sh");
        std::fs::write(&file, b"echo hello")?;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))?;

        assert!(is_executable(&file));
        Ok(())
    }
}

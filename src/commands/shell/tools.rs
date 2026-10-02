use tokio::process::Command;

/// Checks if a given command-line tool exists on the system by using `which` (Unix) or `where` (Windows).
///
/// Returns `true` if the tool is found and the verification command exits successfully, `false` otherwise.
pub async fn validate_tool(tool: &str) -> bool {
    if tool.is_empty() {
        return false;
    }

    #[cfg(unix)]
    let cmd = "which";

    #[cfg(windows)]
    let cmd = "where";

    Command::new(cmd)
        .arg(tool)
        .output()
        .await
        .is_ok_and(|o| o.status.success())
}

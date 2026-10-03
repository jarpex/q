use std::collections::HashSet;
use tokio::process::Command;

const POPULAR_TOOLS: &[&str] = &[
    "ls",
    "cd",
    "pwd",
    "cat",
    "grep",
    "find",
    "sed",
    "awk",
    "sort",
    "uniq",
    "wc",
    "head",
    "tail",
    "cut",
    "tr",
    "xargs",
    "tar",
    "gzip",
    "gunzip",
    "zip",
    "unzip",
    "chmod",
    "chown",
    "mkdir",
    "rmdir",
    "rm",
    "cp",
    "mv",
    "ln",
    "touch",
    "echo",
    "printf",
    "read",
    "test",
    "expr",
    "date",
    "cal",
    "df",
    "du",
    "free",
    "ps",
    "top",
    "kill",
    "killall",
    "ping",
    "curl",
    "wget",
    "ssh",
    "scp",
    "rsync",
    "git",
    "python",
    "python3",
    "node",
    "npm",
    "yarn",
    "cargo",
    "rustc",
    "make",
    "gcc",
    "g++",
    "clang",
    "docker",
    "docker-compose",
    "kubectl",
    "helm",
    "brew",
    "apt",
    "yum",
    "dnf",
    "pacman",
    "systemctl",
    "journalctl",
    "ip",
    "ifconfig",
    "netstat",
    "ss",
    "iptables",
    "firewall",
    "crontab",
    "at",
    "sudo",
    "su",
    "passwd",
    "useradd",
    "usermod",
    "groupadd",
    "tree",
    "jq",
    "yq",
    "ffmpeg",
    "convert",
    "sqlite3",
    "mysql",
    "psql",
    "redis-cli",
    "mongosh",
    "vim",
    "nano",
    "emacs",
    "fd",
    "rg",
    "ripgrep",
    "fzf",
    "bat",
    "exa",
    "htop",
    "btop",
];

/// Represents the system context and environment details used for generating shell commands.
pub struct SystemContext {
    /// Information about the operating system.
    pub os_info: String,
    /// The name of the current shell (e.g., bash, zsh, PowerShell).
    pub shell: String,
    /// Information about the current user and their groups.
    pub user_info: String,
    /// The current working directory.
    pub current_dir: String,
    /// A truncated string representation of the files in the current directory (up to 20 items).
    pub files: String,
    /// A list of available command-line tools found in the system's PATH.
    pub available_tools: Vec<String>,
}

impl SystemContext {
    /// Asynchronously collects system information, user details, and available tools.
    pub async fn collect() -> Self {
        // Run all I/O bound tasks concurrently.
        // get_available_tools is safely offloaded to the blocking thread pool via spawn_blocking.
        let (os_info, user_info, files, available_tools) = tokio::join!(
            Self::get_os_info(),
            Self::get_user_info(),
            Self::get_files(),
            Self::get_available_tools()
        );

        let shell = Self::get_shell();
        let current_dir = Self::get_current_dir();

        Self {
            os_info,
            shell,
            user_info,
            current_dir,
            files,
            available_tools,
        }
    }

    /// Formats the collected system context into a string suitable for an LLM prompt.
    pub fn to_prompt_context(&self) -> String {
        let tools_list = if self.available_tools.is_empty() {
            "No tools detected".to_owned()
        } else {
            self.available_tools.join(", ")
        };

        format!(
            "System context:\n\
             - OS: {}\n\
             - Shell: {}\n\
             - User: {}\n\
             - Current Dir: {}\n\
             - Files: {}\n\
             - Available tools: {}",
            self.os_info, self.shell, self.user_info, self.current_dir, self.files, tools_list
        )
    }

    async fn get_os_info() -> String {
        #[cfg(unix)]
        {
            Command::new("uname")
                .arg("-a")
                .output()
                .await
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map_or_else(|| "Unknown Unix".to_owned(), |s| s.trim().to_owned())
        }

        #[cfg(windows)]
        {
            Command::new("cmd.exe")
                .args(["/C", "ver"])
                .output()
                .await
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map_or_else(|| "Windows".to_owned(), |s| s.trim().to_owned())
        }

        #[cfg(not(any(unix, windows)))]
        {
            "Unknown OS".to_owned()
        }
    }

    fn get_shell() -> String {
        #[cfg(unix)]
        {
            std::env::var("SHELL")
                .ok()
                .and_then(|s| {
                    std::path::Path::new(&s)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "unknown".to_owned())
        }

        #[cfg(windows)]
        {
            if std::env::var("PSModulePath").is_ok() {
                "PowerShell".to_owned()
            } else {
                "CMD".to_owned()
            }
        }

        #[cfg(not(any(unix, windows)))]
        {
            "unknown".to_owned()
        }
    }

    async fn get_user_info() -> String {
        #[cfg(unix)]
        {
            Command::new("id")
                .output()
                .await
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map_or_else(|| "unknown".to_owned(), |s| s.trim().to_owned())
        }

        #[cfg(windows)]
        {
            Command::new("whoami")
                .output()
                .await
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map_or_else(|| "unknown".to_owned(), |s| s.trim().to_owned())
        }

        #[cfg(not(any(unix, windows)))]
        {
            "unknown".to_owned()
        }
    }

    fn get_current_dir() -> String {
        std::env::current_dir().map_or_else(
            |_| "unknown".to_owned(),
            |p| p.to_string_lossy().into_owned(),
        )
    }

    async fn get_files() -> String {
        let Ok(mut entries) = tokio::fs::read_dir(".").await else {
            return String::new();
        };

        let mut files = Vec::with_capacity(20);
        while let Ok(Some(entry)) = entries.next_entry().await {
            if files.len() >= 20 {
                break;
            }
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            files.push(name_str.into_owned());
        }

        files.join("\n")
    }

    async fn get_available_tools() -> Vec<String> {
        // Offload heavy synchronous I/O and CPU work to the blocking thread pool
        // to prevent starving the Tokio async runtime.
        tokio::task::spawn_blocking(Self::get_available_tools_sync)
            .await
            .unwrap_or_default()
    }

    fn get_available_tools_sync() -> Vec<String> {
        let path_var = std::env::var_os("PATH").unwrap_or_default();
        let mut binaries = HashSet::new();

        for dir in std::env::split_paths(&path_var) {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        #[cfg(windows)]
                        let name = name
                            .strip_suffix(".exe")
                            .or_else(|| name.strip_suffix(".cmd"))
                            .or_else(|| name.strip_suffix(".bat"))
                            .unwrap_or(name);

                        binaries.insert(name.to_owned());
                    }
                }
            }
        }

        let mut result = Vec::with_capacity(200);

        // 1. Extract popular tools in O(1) time per lookup using HashSet::remove
        for &tool in POPULAR_TOOLS {
            if binaries.remove(tool) {
                result.push(tool.to_owned());
            }
        }

        // 2. Fill the rest up to 200 tools
        if result.len() < 200 {
            let mut others: Vec<_> = binaries.into_iter().collect();
            // sort_unstable is faster for strings and doesn't allocate extra memory
            others.sort_unstable();

            for tool in others {
                result.push(tool);
                if result.len() >= 200 {
                    break;
                }
            }
        }

        result
    }
}

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
    /// Information about the current user.
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
        let (os_info, user_info, files) = tokio::join!(
            Self::get_os_info(),
            Self::get_user_info(),
            Self::get_files()
        );

        let shell = Self::get_shell();
        let current_dir = Self::get_current_dir();
        let available_tools = Self::get_available_tools();

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
            Command::new("systeminfo")
                .args(["/FO", "CSV", "/NH"])
                .output()
                .await
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map_or_else(
                    || "Windows".to_owned(),
                    |s| s.lines().next().unwrap_or("Windows").to_owned(),
                )
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
        #[cfg(unix)]
        let cmd = "ls -1A 2>/dev/null | head -n 20";

        #[cfg(windows)]
        let cmd = "dir /b 2>nul | findstr /n \".\" | findstr /b \"[1-9][0-9]*:\" | findstr /v \"^2[1-9]\" | cut -d: -f2-";

        #[cfg(not(any(unix, windows)))]
        let cmd = "ls 2>/dev/null | head -n 20";

        Command::new("sh")
            .args(["-c", cmd])
            .output()
            .await
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map_or_else(String::new, |s| s.trim().to_owned())
    }

    fn get_available_tools() -> Vec<String> {
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

        let mut vec: Vec<_> = binaries.into_iter().collect();
        vec.sort();

        if vec.len() > 200 {
            Self::filter_popular_tools(&vec)
        } else {
            vec
        }
    }

    fn filter_popular_tools(vec: &[String]) -> Vec<String> {
        let mut result: Vec<_> = POPULAR_TOOLS
            .iter()
            .filter(|p| vec.iter().any(|v| v == *p))
            .map(|&s| s.to_owned())
            .collect();

        for tool in vec {
            if !result.contains(tool) {
                result.push(tool.clone());
                if result.len() >= 200 {
                    break;
                }
            }
        }

        result
    }
}

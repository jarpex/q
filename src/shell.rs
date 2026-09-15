use crate::config::CookieSet;
use crate::python::ask_gemini_via_python;
use anyhow::Result;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::path::Path;
use tokio::process::Command;

pub(crate) struct SystemContext {
    pub os_info: String,
    pub shell: String,
    pub user_info: String,
    pub current_dir: String,
    pub files: String,
    pub available_tools: Vec<String>,
}

impl SystemContext {
    pub(crate) async fn collect() -> Self {
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
    
    pub(crate) fn to_prompt_context(&self) -> String {
        let tools_list = if self.available_tools.is_empty() {
            "No tools detected".to_string()
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
            self.os_info,
            self.shell,
            self.user_info,
            self.current_dir,
            self.files,
            tools_list
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
                .map_or_else(|| "Unknown Unix".to_string(), |s| s.trim().to_string())
        }
        
        #[cfg(windows)]
        {
            Command::new("systeminfo")
                .args(["/FO", "CSV", "/NH"])
                .output()
                .await
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map_or_else(|| "Windows".to_string(), |s| s.lines().next().unwrap_or("Windows").to_string())
        }
        
        #[cfg(not(any(unix, windows)))]
        {
            "Unknown OS".to_string()
        }
    }
    
    fn get_shell() -> String {
        #[cfg(unix)]
        {
            std::env::var("SHELL")
                .ok()
                .and_then(|s| {
                    Path::new(&s)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(ToString::to_string)
                })
                .unwrap_or_else(|| "unknown".to_string())
        }
        
        #[cfg(windows)]
        {
            if std::env::var("PSModulePath").is_ok() {
                "PowerShell".to_string()
            } else {
                "CMD".to_string()
            }
        }
        
        #[cfg(not(any(unix, windows)))]
        {
            "unknown".to_string()
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
                .map_or_else(|| "unknown".to_string(), |s| s.trim().to_string())
        }
        
        #[cfg(windows)]
        {
            Command::new("whoami")
                .output()
                .await
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map_or_else(|| "unknown".to_string(), |s| s.trim().to_string())
        }
        
        #[cfg(not(any(unix, windows)))]
        {
            "unknown".to_string()
        }
    }
    
    fn get_current_dir() -> String {
        std::env::current_dir()
            .map_or_else(|_| "unknown".to_string(), |p| p.to_string_lossy().to_string())
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
            .map_or_else(String::new, |s| s.trim().to_string())
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
                        
                        binaries.insert(name.to_string());
                    }
                }
            }
        }
        
        let mut vec: Vec<_> = binaries.into_iter().collect();
        vec.sort();
        
        if vec.len() > 200 {
            let popular = [
                "ls", "cd", "pwd", "cat", "grep", "find", "sed", "awk", "sort",
                "uniq", "wc", "head", "tail", "cut", "tr", "xargs", "tar", "gzip",
                "gunzip", "zip", "unzip", "chmod", "chown", "mkdir", "rmdir", "rm",
                "cp", "mv", "ln", "touch", "echo", "printf", "read", "test", "expr",
                "date", "cal", "df", "du", "free", "ps", "top", "kill", "killall",
                "ping", "curl", "wget", "ssh", "scp", "rsync", "git", "python", "python3",
                "node", "npm", "yarn", "cargo", "rustc", "make", "gcc", "g++", "clang",
                "docker", "docker-compose", "kubectl", "helm", "brew", "apt", "yum",
                "dnf", "pacman", "systemctl", "journalctl", "ip", "ifconfig", "netstat",
                "ss", "iptables", "firewall", "crontab", "at", "sudo", "su", "passwd",
                "useradd", "usermod", "groupadd", "tree", "jq", "yq", "ffmpeg", "convert",
                "sqlite3", "mysql", "psql", "redis-cli", "mongosh", "vim", "nano", "emacs",
                "fd", "rg", "ripgrep", "fzf", "bat", "exa", "htop", "btop",
            ];
            
            let mut result: Vec<_> = popular
                .iter()
                .filter(|p| vec.contains(&p.to_string()))
                .map(ToString::to_string)
                .collect();
            
            for tool in vec {
                if !result.contains(&tool) {
                    result.push(tool);
                    if result.len() >= 200 {
                        break;
                    }
                }
            }
            
            result
        } else {
            vec
        }
    }
}

pub(crate) async fn validate_tool(tool: &str) -> bool {
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

fn extract_first_command(cmd: &str) -> &str {
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escape_next = false;
    
    for (i, c) in cmd.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }
        
        if c == '\\' {
            escape_next = true;
            continue;
        }
        
        if c == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }
        
        if c == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }
        
        if !in_single_quote && !in_double_quote && (c == '|' || c == '&' || c == ';') {
            return &cmd[..i];
        }
    }
    
    cmd
}

pub(crate) fn parse_command(response: &str) -> String {
    let mut lines = response.lines().map(str::trim).filter(|l| !l.is_empty());
    
    let Some(first_line) = lines.next() else {
        return String::new();
    };
    
    if first_line.starts_with("```") {
        return lines
            .find(|l| !l.starts_with("```"))
            .unwrap_or("")
            .trim_start_matches("$ ")
            .trim()
            .to_string();
    }
    
    first_line
        .trim_start_matches("$ ")
        .trim()
        .to_string()
}

pub(crate) async fn command_mode(
    python_bin: &Path,
    cookies: &CookieSet,
    query: &str,
    model: &str,
    debug: bool,
) -> Result<()> {
    let ctx = SystemContext::collect().await;
    let system_context = ctx.to_prompt_context();

    let base_prompt = format!(
        "You are a strict {shell} command generator for {os_info}.\n\
         Respond with EXACTLY one raw shell command.\n\
         NO markdown, NO backticks, NO explanation, NO prefix like '$'.\n\
         Provide the COMPLETE command with all necessary flags and options.\n\
         Do NOT truncate or abbreviate the response.\n\n\
         Generate a single, correct {shell} command to fulfill this request:\n\
         Request: \"{query}\"\n\n\
         {system_context}\n\n\
         CRITICAL RULES:\n\
         - Analyze the available tools list and CHOOSE the BEST tool for the task.\n\
         - If modern tools like fd, rg (ripgrep), fzf, bat, exa are available, PREFER them over traditional tools.\n\
         - Generate a command that USES the available tools to accomplish the task, NOT a command to check if tools exist.\n\
         - Use ONLY tools from the available tools list above.\n\
         - Use syntax appropriate for {shell} on {os_info}.\n\
         - Do not invent flags or options. Use standard, well-documented flags.\n\
         - Include ALL necessary flags to fulfill the request completely.\n\
         - Return the FULL command, not abbreviated.\n\
         - The command MUST work when executed. Test it mentally before responding.",
        shell = ctx.shell,
        os_info = ctx.os_info,
        query = query,
        system_context = system_context,
    );

    let title = format!("q ─ command ─ {model}");
    let max_attempts = 3;
    let mut last_error = String::new();

    let spinner = crate::tui::Spinner::start("Thinking...");

    for attempt in 1..=max_attempts {
        if attempt > 1 {
            spinner.set_label(&format!("Retrying ({attempt}/{max_attempts})..."));
        }
        
        let mut prompt = base_prompt.clone();
        if !last_error.is_empty() {
            let _ = write!(
                prompt,
                "\n\nPREVIOUS ATTEMPT FAILED: {last_error}\nGenerate a DIFFERENT command."
            );
        }

        let response = match ask_gemini_via_python(python_bin, cookies, &prompt, model, false, debug).await {
            Ok(r) => r,
            Err(e) => {
                last_error = format!("Request failed: {e}");
                continue;
            }
        };

        let command = parse_command(&response);
        if command.is_empty() {
            last_error = "Generated command was empty or invalid markdown.".to_string();
            continue;
        }

        let first_command = extract_first_command(&command);
        let first_tool = first_command
            .split_whitespace()
            .next()
            .unwrap_or("");

        if !validate_tool(first_tool).await {
            last_error = format!("Tool '{first_tool}' does not exist on this system.");
            continue;
        }

        spinner.stop_and_rewind();

        crate::tui::print_command(&command, &title);

        if crate::tui::copy_to_clipboard(&command) {
            crate::tui::print_copied_message();
        }

        return Ok(());
    }

    spinner.stop_and_rewind();
    crate::tui::print_error(&format!(
        "Failed to generate a valid command after {max_attempts} attempts. Last error: {last_error}"
    ));

    anyhow::bail!("Failed to generate a valid command after {max_attempts} attempts")
}
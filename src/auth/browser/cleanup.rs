//! Cleanup of Windows registry, task scheduler, and shortcut traces left by portable Firefox.

use super::{remove_dir_quiet, remove_file_quiet, system32_cmd, system32_path};

/// Removes the traces that the portable Firefox leaves on the machine.
///
/// Firefox registers itself for autostart (a Run key and a scheduled task),
/// records itself in the compatibility assistant and in the shared Mozilla
/// data directory, and creates a Private Browsing shortcut in the Start Menu.
/// None of that is needed after the login, and leaving it behind makes the
/// tool look as if it had installed a browser.
///
/// Only entries that point at the tool's own Firefox are removed; the
/// Firefox of the user is never touched.
pub fn clean_traces() {
    let mut hashes: Vec<String> = Vec::new();

    // Hashes remembered from earlier runs, so that traces left behind by an
    // interrupted run are still cleaned up.
    if let Ok(stored) = std::fs::read_to_string(hash_list_path()) {
        for line in stored.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !hashes.contains(&trimmed.to_owned()) {
                hashes.push(trimmed.to_owned());
            }
        }
    }

    clean_run_key(&mut hashes);
    clean_scheduled_tasks(&mut hashes);
    clean_appcompat();
    clean_programdata(&hashes);
    clean_shortcuts();

    // Remember the hashes for the next run.
    if let Some(parent) = hash_list_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(hash_list_path(), hashes.join("\n"));
}

/// Path of the file that remembers the hashes of the tool's Firefox installs.
fn hash_list_path() -> std::path::PathBuf {
    crate::config::config_dir().map_or_else(
        |_| std::path::PathBuf::from("trace-hashes.txt"),
        |directory| directory.join("firefox").join("trace-hashes.txt"),
    )
}

/// Reports whether the path belongs to the tool's own Firefox.
pub(crate) fn is_ours_path(path: &str) -> bool {
    let lower = path.to_lowercase();

    if let Some(prog_files) = std::env::var_os("ProgramFiles") {
        let pf = prog_files.to_string_lossy().to_lowercase();
        if !pf.is_empty() && lower.starts_with(&pf) {
            return false;
        }
    }
    if let Some(sys_root) = std::env::var_os("SystemRoot") {
        let sr = sys_root.to_string_lossy().to_lowercase();
        if !sr.is_empty() && lower.starts_with(&sr) {
            return false;
        }
    }

    // Never match common system installation roots.
    if lower.starts_with(r"c:\program files")
        || lower.starts_with(r"c:\program files (x86)")
        || lower.starts_with(r"c:\windows")
    {
        return false;
    }

    if let Ok(config) = crate::config::config_dir() {
        let prefix = config.join("firefox").to_string_lossy().to_lowercase();
        if !prefix.is_empty() && lower.contains(&prefix) {
            return true;
        }
    }

    let base = std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    base.starts_with("q-firefox-") || base.starts_with("q-mn-") || base == "q-firefox.exe"
}

/// Removes the Run key entries that Firefox created for the tool's browser.
fn clean_run_key(hashes: &mut Vec<String>) {
    let Ok(key) =
        windows_registry::CURRENT_USER.open(r"Software\Microsoft\Windows\CurrentVersion\Run")
    else {
        return;
    };

    let Ok(values) = key.values() else {
        return;
    };

    for (name, value) in values {
        if !name.starts_with("Mozilla-Firefox-") {
            continue;
        }

        let data = String::try_from(value).unwrap_or_default();
        if !is_ours_path(&data) {
            continue;
        }

        let hash = name
            .strip_prefix("Mozilla-Firefox-")
            .unwrap_or_default()
            .to_owned();
        if !hash.is_empty() && !hashes.contains(&hash) {
            hashes.push(hash);
        }

        let _ = key.remove_value(&name);
    }
}

/// Removes the scheduled tasks that Firefox created for the tool's browser.
///
/// The task definitions are read directly from the XML files under the
/// Mozilla task folder, so the parsing does not depend on the localized
/// output of `schtasks.exe`.
fn clean_scheduled_tasks(hashes: &mut Vec<String>) {
    let tasks_root = std::path::Path::new(r"C:\Windows\System32\Tasks\Mozilla");

    // Walk the task folder recursively so tasks in subfolders are found too.
    let mut xml_files: Vec<std::path::PathBuf> = Vec::new();
    let mut stack = vec![tasks_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("xml") {
                xml_files.push(path);
            }
        }
    }

    let mut task_names: Vec<String> = Vec::new();
    for path in xml_files {
        let Ok(xml) = std::fs::read_to_string(&path) else {
            continue;
        };

        let command = extract_xml_tag(&xml, "Command");
        if !is_ours_path(&command) {
            continue;
        }

        // The task name is the path of the file relative to the Tasks folder.
        let name = path
            .strip_prefix(r"C:\Windows\System32\Tasks")
            .ok()
            .map_or_else(
                || {
                    path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()
                },
                |relative| relative.to_string_lossy().replace('/', "\\"),
            );

        task_names.push(name);
    }

    for name in task_names {
        // Collect the hash from the task name (last whitespace token).
        if let Some(hash) = name.split_whitespace().last() {
            if hash.chars().all(|character| character.is_ascii_hexdigit())
                && !hashes.contains(&hash.to_owned())
            {
                hashes.push(hash.to_owned());
            }
        }

        let _ = system32_cmd("schtasks.exe")
            .args(["/delete", "/tn", &name, "/f"])
            .output();
    }
}

/// Extracts the text content of the first `<tag ...>...</tag>` element.
///
/// Handles opening tags that carry attributes (for example
/// `<Command xmlns="...">`), which the Windows Task Scheduler may emit.
fn extract_xml_tag(document: &str, tag: &str) -> String {
    let open_prefix = format!("<{tag}");
    let close = format!("</{tag}>");

    let mut search = 0;
    while let Some(relative) = document.get(search..).and_then(|s| s.find(&open_prefix)) {
        let start = search + relative;
        let after_prefix = start + open_prefix.len();

        // The character after the tag name must be '>' or whitespace, so that
        // `<Commander>` does not match `<Command`.
        let Some(next) = document.get(after_prefix..).and_then(|s| s.chars().next()) else {
            break;
        };
        if !matches!(next, '>' | ' ' | '\t' | '\n' | '\r') {
            search = after_prefix;
            continue;
        }

        // Find the closing '>' of the opening tag, honouring quoted attribute
        // values so a '>' inside an attribute does not terminate the scan.
        let Some(open_end_relative) = find_tag_end(document, after_prefix) else {
            break;
        };
        let content_start = open_end_relative + 1;

        let Some(end_relative) = document.get(content_start..).and_then(|s| s.find(&close)) else {
            search = content_start;
            continue;
        };

        let content = document
            .get(content_start..content_start + end_relative)
            .unwrap_or("");
        return content.trim().to_owned();
    }

    String::new()
}

/// Returns the byte index of the '>' that closes the opening tag whose name
/// has already been consumed up to `from`. Quoted attribute values are skipped
/// so a '>' inside them is not mistaken for the terminator.
fn find_tag_end(document: &str, from: usize) -> Option<usize> {
    let mut in_single = false;
    let mut in_double = false;
    for (offset, ch) in document.get(from..)?.char_indices() {
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '>' if !in_single && !in_double => return Some(from + offset),
            _ => {}
        }
    }
    None
}

/// Removes the compatibility-assistant entries for the tool's browser.
fn clean_appcompat() {
    let key = r"HKCU\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Compatibility Assistant\Store";

    let Ok(output) = system32_cmd("reg.exe").args(["query", key]).output() else {
        return;
    };

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        // The value name is the part before the type token.
        let trimmed = line.trim();
        let Some((name, _)) = trimmed
            .split_once("REG_BINARY")
            .or_else(|| trimmed.split_once("REG_SZ"))
        else {
            continue;
        };

        let path = name.trim();
        if path.is_empty() || !is_ours_path(path) {
            continue;
        }

        let _ = system32_cmd("reg.exe")
            .args(["delete", key, "/v", path, "/f"])
            .output();
    }
}

/// Removes the per-browser files that Firefox leaves in the shared Mozilla
/// data directory for the tool's browser.
fn clean_programdata(hashes: &[String]) {
    if hashes.is_empty() {
        return;
    }

    let program_data = std::path::Path::new(r"C:\ProgramData");
    let Ok(entries) = std::fs::read_dir(program_data) else {
        return;
    };

    for entry in entries.filter_map(Result::ok) {
        let dir_name = entry.file_name();
        let dir_text = dir_name.to_string_lossy();
        if !dir_text.starts_with("Mozilla-") {
            continue;
        }

        let base = entry.path();
        for hash in hashes {
            // Files keyed by the hash.
            remove_file_quiet(&base.join(format!("profile_count_{hash}.json")));
            remove_file_quiet(&base.join(format!("UpdateLock-{hash}")));
            if let Ok(dir_entries) = std::fs::read_dir(&base) {
                for item in dir_entries.filter_map(Result::ok) {
                    let item_file = item.file_name();
                    let item_text = item_file.to_string_lossy();
                    if item_text.starts_with(&format!("uninstall_ping_{hash}_")) {
                        remove_file_quiet(&item.path());
                    }
                }
            }

            // The updates directory for the hash.
            remove_dir_quiet(&base.join("updates").join(hash));
        }
    }
}

/// Removes Start Menu, Desktop and Quick Launch shortcuts that point at the
/// tool's own Firefox.
///
/// Firefox creates a `Firefox Private Browsing.lnk` shortcut in the per-user
/// Start Menu on its first run. Such shortcuts are read through the Windows
/// shell so that only shortcuts whose target belongs to the tool are removed;
/// shortcuts of any other browser are left untouched.
fn clean_shortcuts() {
    let Ok(config) = crate::config::config_dir() else {
        return;
    };
    let firefox_dir = config.join("firefox");

    let script = r"
$sh = New-Object -ComObject WScript.Shell
$dirs = @(
  [Environment]::GetFolderPath('StartMenu'),
  [Environment]::GetFolderPath('CommonStartMenu'),
  [Environment]::GetFolderPath('Desktop'),
  [Environment]::GetFolderPath('CommonDesktopDirectory'),
  ([Environment]::GetFolderPath('ApplicationData') + '\Microsoft\Internet Explorer\Quick Launch')
)
foreach ($d in $dirs) {
  if (-not $d -or -not (Test-Path -LiteralPath $d)) { continue }
  Get-ChildItem -Path $d -Recurse -Filter *.lnk -Force -ErrorAction SilentlyContinue |
    ForEach-Object {
      try {
        $s = $sh.CreateShortcut($_.FullName)
        $target = $s.TargetPath
        if ($target) {
          $lower = $target.ToLower()
          if ($lower.StartsWith('c:\program files') -or
              $lower.StartsWith('c:\program files (x86)') -or
              $lower.StartsWith('c:\windows')) {
            continue
          }
          $ours = $false
          if ($env:Q_FIREFOX_PATH -and $env:Q_FIREFOX_PATH.Trim() -ne '' -and
              $lower.Contains($env:Q_FIREFOX_PATH.ToLower())) {
            $ours = $true
          }
          $base = [System.IO.Path]::GetFileName($target).ToLower()
          if ($base -eq 'q-firefox.exe' -or
              $base -like 'q-firefox-*' -or
              $base -like 'q-mn-*') {
            $ours = $true
          }
          if ($ours) {
            Remove-Item -LiteralPath $_.FullName -Force -ErrorAction SilentlyContinue
          }
        }
      } catch { }
    }
}
";
    let powershell = system32_path(r"WindowsPowerShell\v1.0\powershell.exe");
    let mut command = std::process::Command::new(powershell);
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]);
    command.env("Q_FIREFOX_PATH", firefox_dir.to_string_lossy().to_string());
    let _ = command.output();
}

#[cfg(test)]
mod tests {
    use super::is_ours_path;

    #[test]
    fn is_ours_path_rejects_system_locations() {
        assert!(!is_ours_path(
            r"C:\Program Files\Mozilla Firefox\firefox.exe"
        ));
        assert!(!is_ours_path(
            r"C:\Program Files (x86)\Mozilla Firefox\firefox.exe"
        ));
        assert!(!is_ours_path(r"C:\Windows\System32\cmd.exe"));
        assert!(is_ours_path(
            r"C:\Users\user\.config\q\firefox\158.0.1\core\q-firefox-1.exe"
        ));
    }
}

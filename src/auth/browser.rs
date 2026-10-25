//! Provisions the portable Mozilla Firefox that the login flow drives.
//!
//! The tool does not use the Firefox of the user: it downloads the official
//! installer of a pinned Firefox release, checks its SHA-256 against a pinned
//! digest and unpacks the 7-Zip payload that the installer carries. No
//! installer, no MSI and no external archiver runs on the machine, and the
//! Firefox of the user is never touched.
//!
//! The unpacked browser lives in the configuration directory of the tool, and
//! every login gets a profile of its own, so the session of the user cannot
//! rotate the cookies that the tool is about to read.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use crate::config::config_dir;

/// Version of Firefox the tool pins.
pub const FIREFOX_VERSION: &str = "158.0.1";

/// Address of the official installer of the pinned version.
pub const FIREFOX_URL: &str =
    "https://ftp.mozilla.org/pub/firefox/releases/158.0.1/win64/en-US/Firefox%20Setup%20158.0.1.exe";

/// SHA-256 of the official installer of the pinned version.
pub const FIREFOX_SHA256: &str = "3259a2f8508e58cf2e383b6c99bfc64d6ddc86d1407b7c2f86f660af955b5d10";

/// Signature that starts a 7-Zip archive.
const ARCHIVE_SIGNATURE: [u8; 6] = [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];

/// Name of the browser executable inside the unpacked archive.
const EXECUTABLE_NAME: &str = "core/firefox.exe";

/// Name of the file that holds the version of the unpacked browser.
const APPLICATION_INI: &str = "core/application.ini";

/// A portable Firefox that belongs to the tool.
#[derive(Debug)]
pub struct Firefox {
    root: PathBuf,
}

impl Firefox {
    /// Returns the directory the browser is unpacked into.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration directory cannot be determined.
    pub fn install_dir() -> Result<PathBuf> {
        Ok(config_dir()?.join("firefox").join(FIREFOX_VERSION))
    }

    /// Makes sure that the portable Firefox is unpacked and ready to run.
    ///
    /// The browser is downloaded once and kept in the configuration directory
    /// of the tool. Later logins reuse it.
    ///
    /// # Errors
    ///
    /// Returns an error if the browser cannot be downloaded, does not match
    /// the pinned digest, cannot be unpacked, or does not carry the pinned
    /// version.
    #[allow(clippy::print_stdout)]
    pub fn ensure_installed() -> Result<Self> {
        let firefox = Self {
            root: Self::install_dir()?,
        };

        if firefox.is_installed() {
            return Ok(firefox);
        }

        fs::create_dir_all(&firefox.root)
            .with_context(|| format!("cannot create {}", firefox.root.display()))?;

        let installer = firefox.root.join("installer.exe");
        println!("Downloading the browser, this takes a moment...");
        download_installer(&installer)?;
        println!("Checking the browser...");
        verify_checksum(&installer)?;
        println!("Unpacking the browser...");
        unpack(&installer, &firefox.root)?;
        remove_file_quiet(&installer);

        if !firefox.is_installed() {
            bail!("the downloaded installer did not contain the browser");
        }

        firefox.verify_version()?;
        Ok(firefox)
    }

    /// Reports whether the browser is unpacked and usable.
    fn is_installed(&self) -> bool {
        self.executable().is_file()
    }

    /// Reports whether the browser is already unpacked.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration directory cannot be determined.
    pub fn is_installed_at_default_location() -> Result<bool> {
        let firefox = Self {
            root: Self::install_dir()?,
        };

        Ok(firefox.is_installed())
    }

    /// Returns the directory the browser is unpacked into.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the directory that holds the browser and its libraries.
    pub fn core_dir(&self) -> PathBuf {
        self.root.join("core")
    }

    /// Returns the path of the browser executable.
    pub fn executable(&self) -> PathBuf {
        self.root.join("core").join("firefox.exe")
    }

    /// Returns the path of a copy of the browser under a name of its own.
    ///
    /// The copy has to stay next to the original executable: the browser
    /// loads its libraries through relative paths. The unique name keeps the
    /// process of the tool apart from the processes of the user, so that
    /// closing the browser of the tool can never reach the Firefox of the
    /// user.
    ///
    /// # Errors
    ///
    /// Returns an error if the copy cannot be created.
    pub fn unique_executable(&self) -> Result<PathBuf> {
        let target = self
            .core_dir()
            .join(format!("q-firefox-{}.exe", unique_suffix()));

        fs::copy(self.executable(), &target)
            .with_context(|| format!("cannot copy the browser to {}", target.display()))?;

        Ok(target)
    }

    /// Checks that the unpacked browser carries the pinned version.
    ///
    /// # Errors
    ///
    /// Returns an error if the version file is missing or names another
    /// version than the pinned one.
    fn verify_version(&self) -> Result<()> {
        let path = self.root.join(APPLICATION_INI);
        let content =
            fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;

        let version = content
            .lines()
            .find_map(|line| line.trim().strip_prefix("Version="))
            .map(str::trim);

        match version {
            Some(found) if found == FIREFOX_VERSION => Ok(()),
            Some(found) => bail!(
                "the downloaded browser is version {found}, but version {FIREFOX_VERSION} was expected"
            ),
            None => bail!("the downloaded browser does not name its version"),
        }
    }
}

/// Creates a profile of its own for one login attempt.
///
/// The profile holds the session of the sign-in, so it lives next to the
/// browser and is removed once the cookies have been read.
///
/// # Errors
///
/// Returns an error if the profile directory cannot be created.
pub fn create_profile() -> Result<PathBuf> {
    let profile = config_dir()?
        .join("firefox")
        .join("profiles")
        .join(format!("login-{}", unique_suffix()));

    fs::create_dir_all(&profile).with_context(|| format!("cannot create {}", profile.display()))?;

    fs::write(profile.join("user.js"), PROFILE_PREFS)
        .with_context(|| format!("cannot write the preferences of {}", profile.display()))?;

    Ok(profile)
}

/// Removes the profiles of the logins that were not finished.
///
/// A login that was interrupted leaves its profile behind, and that profile
/// holds the cookies of the sign-in. The profiles are of no use to anybody
/// else, so they are removed at the start of the next login.
///
/// # Errors
///
/// Returns an error if the profiles cannot be listed.
pub fn remove_stale_profiles() -> Result<()> {
    let directory = config_dir()?.join("firefox").join("profiles");

    let Ok(entries) = fs::read_dir(&directory) else {
        return Ok(());
    };

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            remove_dir_quiet(&path);
        }
    }

    Ok(())
}

/// Removes a profile that is not needed any more.
///
/// # Errors
///
/// Returns an error if the profile cannot be removed.
pub fn remove_profile(profile: &Path) -> Result<()> {
    if !profile.exists() {
        return Ok(());
    }

    fs::remove_dir_all(profile).with_context(|| format!("cannot remove {}", profile.display()))
}

/// Preferences of the profile of the login.
///
/// They turn off the pages that Firefox shows on its first start and the
/// warning about the default browser, and they shorten the interval in which
/// Firefox writes its session store, so that the cookies of the sign-in
/// become visible while the browser is still running.
///
/// The preferences that keep the browser from reaching out to the network
/// also make it start faster: the first start of a fresh profile otherwise
/// waits for the lists of safe browsing and for the region of the machine.
const PROFILE_PREFS: &str = r#"
user_pref("browser.aboutwelcome.enabled", false);
user_pref("browser.sessionstore.interval", 3000);
user_pref("browser.shell.checkDefaultBrowser", false);
user_pref("browser.startup.homepage", "about:blank");
user_pref("browser.tabs.warnOnClose", false);
user_pref("browser.warnOnQuit", false);
user_pref("datareporting.policy.dataSubmissionEnabled", false);
user_pref("toolkit.telemetry.enabled", false);
user_pref("app.update.auto", false);
user_pref("app.update.enabled", false);
user_pref("browser.contentblocking.report.hide_vpn_banner", true);
user_pref("browser.discovery.enabled", false);
user_pref("browser.newtabpage.activity-stream.feeds.telemetry", false);
user_pref("browser.region.network.url", "");
user_pref("browser.region.update.enabled", false);
user_pref("browser.safebrowsing.malware.enabled", false);
user_pref("browser.safebrowsing.phishing.enabled", false);
user_pref("browser.safebrowsing.blockedURIs.enabled", false);
user_pref("browser.safebrowsing.downloads.enabled", false);
user_pref("browser.search.update", false);
user_pref("browser.uitour.enabled", false);
user_pref("captivedetect.canonicalURL", "");
user_pref("network.captive-portal-service.enabled", false);
user_pref("network.connectivity-service.enabled", false);
user_pref("network.captive-portal-service.timeout", 1);
user_pref("network.dns.disablePrefetch", true);
user_pref("network.prefetch-next", false);
user_pref("network.predictor.enabled", false);
user_pref("services.settings.server", "");
"#;

/// Removes the traces that the portable Firefox leaves on the machine.
///
/// Firefox registers itself for autostart (a Run key and a scheduled task)
/// and records itself in the compatibility assistant and in the shared
/// Mozilla data directory. None of that is needed after the login, and
/// leaving it behind makes the tool look as if it had installed a browser.
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

    // Remember the hashes for the next run.
    if let Some(parent) = hash_list_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(
        hash_list_path(),
        hashes.join(
            "
",
        ),
    );
}

/// Path of the file that remembers the hashes of the tool's Firefox installs.
fn hash_list_path() -> std::path::PathBuf {
    crate::config::config_dir().map_or_else(
        |_| std::path::PathBuf::from("trace-hashes.txt"),
        |directory| directory.join("firefox").join("trace-hashes.txt"),
    )
}

/// Reports whether the path belongs to the tool's own Firefox.
fn is_ours_path(path: &str) -> bool {
    let lower = path.to_lowercase();

    if let Ok(config) = crate::config::config_dir() {
        let prefix = config.join("firefox").to_string_lossy().to_lowercase();
        if lower.contains(&prefix) {
            return true;
        }
    }

    let base = std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    base.starts_with("q-firefox-") || base.starts_with("q-mn-") || base == "q-firefox.exe"
}

/// Removes a file, logging failures other than "not found".
#[allow(clippy::print_stderr)]
fn remove_file_quiet(path: &std::path::Path) {
    if let Err(error) = std::fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            eprintln!("warning: cannot remove {}: {error}", path.display());
        }
    }
}

/// Removes a directory tree, logging failures other than "not found".
#[allow(clippy::print_stderr)]
fn remove_dir_quiet(path: &std::path::Path) {
    if let Err(error) = std::fs::remove_dir_all(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            eprintln!("warning: cannot remove {}: {error}", path.display());
        }
    }
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

        let _ = std::process::Command::new("schtasks")
            .args(["/delete", "/tn", &name, "/f"])
            .output();
    }
}

/// Extracts the text of an XML tag from a document.
fn extract_xml_tag(document: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");

    let Some(start) = document.find(&open) else {
        return String::new();
    };
    let Some(rest) = document.get(start + open.len()..) else {
        return String::new();
    };
    let Some(end) = rest.find(&close) else {
        return String::new();
    };

    rest.get(..end).unwrap_or("").trim().to_owned()
}

/// Removes the compatibility-assistant entries for the tool's browser.
fn clean_appcompat() {
    let key = r"HKCU\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Compatibility Assistant\Store";

    let Ok(output) = std::process::Command::new("reg")
        .args(["query", key])
        .output()
    else {
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

        let _ = std::process::Command::new("reg")
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
///
/// Downloads the Firefox installer.
///
/// The download runs through the curl.exe that ships with Windows, at its
/// absolute path, so it does not depend on the Python interpreter of the
/// user and cannot be redirected by a hostile executable earlier on PATH.
///
/// # Errors
///
/// Returns an error if curl cannot be run or the download fails.
fn download_installer(destination: &Path) -> Result<()> {
    // Use the curl.exe that ships with Windows 10 and later, at its absolute
    // path, so the download does not depend on the Python interpreter of the
    // user and cannot be redirected by a hostile executable earlier on PATH.
    let curl = Path::new(r"C:\Windows\System32\curl.exe");

    let output = std::process::Command::new(curl)
        .arg("--location")
        .arg("--silent")
        .arg("--show-error")
        .arg("--fail")
        .arg("--max-time")
        .arg("300")
        .arg("--user-agent")
        .arg("q-firefox-installer")
        .arg("--output")
        .arg(destination)
        .arg(FIREFOX_URL)
        .output()
        .with_context(|| format!("cannot run {}", curl.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to download the browser: {stderr}");
    }

    Ok(())
}

/// Checks the digest of the downloaded installer.
///
/// # Errors
///
/// Returns an error if the file cannot be read or does not match the pinned
/// digest.
fn verify_checksum(path: &Path) -> Result<()> {
    let mut file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;

    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];

    loop {
        let read = file
            .read(&mut buffer)
            .context("cannot read the installer")?;
        if read == 0 {
            break;
        }
        digest.update(buffer.get(..read).unwrap_or_default());
    }

    let found = digest
        .finalize()
        .iter()
        .fold(String::new(), |mut text, byte| {
            use std::fmt::Write;
            let _ = write!(text, "{byte:02x}");
            text
        });

    if found != FIREFOX_SHA256 {
        bail!(
            "the downloaded browser has the digest {found}, but the digest of the \
             pinned installer is {FIREFOX_SHA256}"
        );
    }

    Ok(())
}

/// Unpacks the 7-Zip payload that the installer carries.
///
/// The installer is a program with an archive appended to it. The archive is
/// found by looking for the signature of a 7-Zip archive and is unpacked with
/// the archive reader of the project, so no external program is needed.
///
/// # Errors
///
/// Returns an error if the archive cannot be found or unpacked.
fn unpack(installer: &Path, destination: &Path) -> Result<()> {
    let file =
        File::open(installer).with_context(|| format!("cannot open {}", installer.display()))?;
    let length = file
        .metadata()
        .with_context(|| format!("cannot read the size of {}", installer.display()))?
        .len();

    let mut failure = None;

    for offset in archive_offsets(&file, length)? {
        match unpack_at(&file, length, offset, destination) {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(error) => failure = Some(error),
        }
    }

    match failure {
        Some(error) => Err(error).context("the installer does not hold a readable archive"),
        None => bail!("the installer does not hold a readable archive"),
    }
}

/// Returns the offsets in the file where an archive may start.
fn archive_offsets(file: &File, _length: u64) -> Result<Vec<u64>> {
    // Read the installer once. A single read is simpler and faster than the
    // previous chunked scan with manual overlap handling, and avoids the
    // allocations of carrying a tail window between chunks.
    use std::io::{Read, Seek, SeekFrom};

    let mut reader = file;
    reader
        .seek(SeekFrom::Start(0))
        .context("cannot seek the installer")?;

    let mut data = Vec::new();
    reader
        .read_to_end(&mut data)
        .context("cannot read the installer")?;

    let mut offsets = Vec::new();
    let mut start = 0;
    while let Some(found) = find_signature(&data, start) {
        offsets.push(found as u64);
        start = found + 1;
    }

    Ok(offsets)
}

/// Finds the next archive signature in the window, starting at the offset.
fn find_signature(window: &[u8], start: usize) -> Option<usize> {
    let tail = window.get(start..)?;
    let found = tail
        .windows(ARCHIVE_SIGNATURE.len())
        .position(|part| part == ARCHIVE_SIGNATURE)?;

    Some(start + found)
}

/// Unpacks the archive that starts at the given offset.
///
/// Returns whether the archive held the browser.
#[allow(clippy::result_large_err)]
fn unpack_at(file: &File, length: u64, offset: u64, destination: &Path) -> Result<bool> {
    if offset >= length {
        return Ok(false);
    }

    let mut source = file.try_clone().context("cannot read the installer")?;
    let view = ArchiveView {
        file: &mut source,
        base: offset,
        length: length - offset,
        position: 0,
    };

    let mut reader = sevenz_rust2::ArchiveReader::new(view, sevenz_rust2::Password::empty())
        .map_err(|error| anyhow::anyhow!("cannot read the archive: {error}"))?;

    if !reader.archive().files.iter().any(is_executable) {
        return Ok(false);
    }

    let target = destination.to_path_buf();
    let mut failure = None;

    reader
        .for_each_entries(
            |entry, entry_reader| match write_entry(&target, entry, entry_reader) {
                Ok(()) => Ok(true),
                Err(error) => {
                    failure = Some(error);
                    Ok(false)
                }
            },
        )
        .map_err(|error| anyhow::anyhow!("cannot unpack the archive: {error}"))?;

    if let Some(error) = failure {
        return Err(error);
    }

    Ok(true)
}

/// Reports whether the entry of the archive is the browser executable.
fn is_executable(entry: &sevenz_rust2::ArchiveEntry) -> bool {
    normalize(&entry.name) == EXECUTABLE_NAME
}

/// Writes one entry of the archive into the destination directory.
///
/// # Errors
///
/// Returns an error if the entry cannot be written.
fn write_entry(
    destination: &Path,
    entry: &sevenz_rust2::ArchiveEntry,
    reader: &mut dyn Read,
) -> Result<()> {
    let Some(relative) = safe_relative_path(&entry.name) else {
        return Ok(());
    };

    let target = destination.join(&relative);

    if entry.is_directory {
        fs::create_dir_all(&target)
            .with_context(|| format!("cannot create {}", target.display()))?;
        return Ok(());
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }

    let mut file =
        File::create(&target).with_context(|| format!("cannot create {}", target.display()))?;

    std::io::copy(reader, &mut file)
        .with_context(|| format!("cannot write {}", target.display()))?;

    Ok(())
}

/// Turns the name of an entry into a relative path inside the destination.
///
/// Returns `None` for entries that would leave the destination directory.
fn safe_relative_path(name: &str) -> Option<PathBuf> {
    let normalized = normalize(name);
    let relative = Path::new(&normalized);

    if relative.is_absolute() {
        return None;
    }

    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                let name = part.to_string_lossy();
                if !is_safe_windows_name(&name) {
                    return None;
                }
            }
            _ => return None,
        }
    }

    if relative.as_os_str().is_empty() {
        return None;
    }

    Some(relative.to_path_buf())
}

/// Reports whether a single path component is a safe Windows file name.
///
/// Rejects names that contain characters that are illegal in Windows file
/// names (including the colon, which would create an NTFS alternate data
/// stream), reserved device names, and trailing spaces or dots.
fn is_safe_windows_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }

    // Characters that are illegal in Windows file names. The colon is the
    // separator for NTFS alternate data streams, so a name such as
    // "firefox.exe:evil" would smuggle data into an alternate stream.
    if name
        .chars()
        .any(|ch| matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
    {
        return false;
    }

    // Control characters are also illegal.
    if name.chars().any(|ch| (ch as u32) < 0x20) {
        return false;
    }

    // Trailing spaces or dots are stripped by Windows and can be used to
    // bypass filters.
    if name.ends_with(' ') || name.ends_with('.') {
        return false;
    }

    // Reserved device names (case-insensitive), with or without an extension.
    let stem = name.split('.').next().unwrap_or(name);
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        return false;
    }

    true
}

/// Turns the separators of an entry name into slashes.
fn normalize(name: &str) -> String {
    name.replace('\\', "/")
}

/// A view on the part of the installer that holds the archive.
///
/// The archive reader expects a stream that starts with the archive, so the
/// view presents the tail of the installer as a file of its own.
struct ArchiveView<'a> {
    file: &'a mut File,
    base: u64,
    length: u64,
    position: u64,
}

impl Read for ArchiveView<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self
            .file
            .seek(SeekFrom::Start(self.base + self.position))
            .and_then(|_| self.file.read(buffer))?;
        self.position += read as u64;
        Ok(read)
    }
}

impl Seek for ArchiveView<'_> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let target = match position {
            SeekFrom::Start(offset) => offset.cast_signed(),
            SeekFrom::End(offset) => self.length.cast_signed().saturating_add(offset),
            SeekFrom::Current(offset) => self.position.cast_signed().saturating_add(offset),
        };

        if target < 0 {
            return Err(std::io::Error::other(
                "seek before the start of the archive",
            ));
        }

        self.position = u64::try_from(target)
            .unwrap_or(self.length)
            .min(self.length);
        Ok(self.position)
    }
}

/// Returns a suffix that no other file of this process uses.
fn unique_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());

    format!("{}-{nanos:x}", std::process::id())
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{
        is_executable, normalize, remove_stale_profiles, safe_relative_path, unique_suffix,
        EXECUTABLE_NAME, FIREFOX_SHA256, FIREFOX_URL, FIREFOX_VERSION, PROFILE_PREFS,
    };
    use crate::config::config_dir;
    use sevenz_rust2::ArchiveEntry;

    /// Builds an entry of an archive.
    fn entry(name: &str) -> ArchiveEntry {
        ArchiveEntry {
            name: name.to_owned(),
            has_stream: true,
            is_directory: false,
            ..ArchiveEntry::default()
        }
    }

    #[test]
    fn pins_the_official_installer() {
        assert_eq!(FIREFOX_VERSION, "158.0.1");
        assert!(FIREFOX_URL.contains(FIREFOX_VERSION));
        assert_eq!(FIREFOX_SHA256.len(), 64);
        assert!(FIREFOX_SHA256
            .chars()
            .all(|character| character.is_ascii_hexdigit()));
    }

    #[test]
    fn finds_the_browser_in_the_archive() {
        assert!(is_executable(&entry("core\\firefox.exe")));
        assert!(!is_executable(&entry("core\\firefox.dll")));
    }

    #[test]
    fn keeps_the_entries_inside_the_destination() {
        assert_eq!(
            safe_relative_path("core\\firefox.exe"),
            Some(std::path::PathBuf::from("core/firefox.exe"))
        );
        assert_eq!(safe_relative_path("..\\evil.exe"), None);
        assert_eq!(safe_relative_path("core\\..\\..\\evil.exe"), None);
        assert_eq!(safe_relative_path("C:\\evil.exe"), None);
    }

    #[test]
    fn turns_the_separators_into_slashes() {
        assert_eq!(normalize("core\\browser\\omni.ja"), "core/browser/omni.ja");
    }

    #[test]
    fn the_profile_silences_the_browser() {
        assert!(PROFILE_PREFS.contains("browser.aboutwelcome.enabled"));
        assert!(PROFILE_PREFS.contains("browser.warnOnQuit"));
    }

    #[test]
    fn the_profile_keeps_the_browser_off_the_network() {
        // The first start of a fresh profile waits for the lists of safe
        // browsing and for the region of the machine. Turning those off
        // makes the window appear much sooner.
        assert!(PROFILE_PREFS.contains("browser.safebrowsing.malware.enabled"));
        assert!(PROFILE_PREFS.contains("browser.region.update.enabled"));
        assert!(PROFILE_PREFS.contains("network.captive-portal-service.enabled"));
        assert!(PROFILE_PREFS.contains("app.update.enabled"));
    }

    #[test]
    fn removes_the_profiles_of_the_logins_that_were_interrupted() -> anyhow::Result<()> {
        let stale = config_dir()?
            .join("firefox")
            .join("profiles")
            .join("login-stale");
        std::fs::create_dir_all(&stale)?;
        std::fs::write(stale.join("cookies.sqlite"), b"cookies")?;

        remove_stale_profiles()?;

        assert!(!stale.exists(), "the stale profile is still there");
        Ok(())
    }

    #[test]
    fn the_executable_sits_in_the_core_directory() {
        assert_eq!(EXECUTABLE_NAME, "core/firefox.exe");
    }

    #[test]
    fn the_suffix_differs_between_two_calls() {
        assert_ne!(unique_suffix(), unique_suffix());
    }
}

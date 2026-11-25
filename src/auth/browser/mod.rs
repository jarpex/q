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

mod cleanup;
mod installer;
mod profile;

pub use cleanup::clean_traces;
pub use profile::{create_profile, remove_profile, remove_stale_profiles};

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::config_dir;

/// Version of Firefox the tool pins.
pub const FIREFOX_VERSION: &str = "158.0.1";

/// Address of the official installer of the pinned version.
pub const FIREFOX_URL: &str =
    "https://ftp.mozilla.org/pub/firefox/releases/158.0.1/win64/en-US/Firefox%20Setup%20158.0.1.exe";

/// SHA-256 of the official installer of the pinned version.
pub const FIREFOX_SHA256: &str = "3259a2f8508e58cf2e383b6c99bfc64d6ddc86d1407b7c2f86f660af955b5d10";

/// Name of the file that holds the version of the unpacked browser.
const APPLICATION_INI: &str = "core/application.ini";

/// The source of the browser used for authentication.
#[derive(Debug)]
pub enum Browser {
    /// A system Firefox installed on the machine.
    System {
        /// Path to the system `firefox.exe`.
        executable: PathBuf,
    },
    /// The tool's portable Firefox unpacked in the configuration directory.
    Portable(Firefox),
}

/// Browser executable prepared for authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedExecutable {
    /// Path to the binary to execute.
    pub path: PathBuf,
    /// True when the executable is a temporary copy removed on exit.
    pub is_temporary: bool,
}

impl Browser {
    /// Reports whether this browser is the tool's portable installation.
    #[must_use]
    pub const fn is_portable(&self) -> bool {
        matches!(self, Self::Portable(_))
    }

    /// Returns the executable to run for this login attempt.
    ///
    /// For the portable Firefox, creates a uniquely named copy next to the original
    /// executable so it does not conflict with user processes. For system Firefox,
    /// returns the system executable directly without copying.
    ///
    /// # Errors
    ///
    /// Returns an error if the portable executable copy cannot be created.
    pub fn launch_executable(&self) -> Result<PathBuf> {
        match self {
            Self::System { executable } => Ok(executable.clone()),
            Self::Portable(firefox) => firefox.unique_executable(),
        }
    }

    /// Returns the executable to run and whether it is a temporary copy to remove.
    ///
    /// # Errors
    ///
    /// Returns an error if the portable executable copy cannot be created.
    pub fn prepare_executable(&self) -> Result<PreparedExecutable> {
        match self {
            Self::System { executable } => Ok(PreparedExecutable {
                path: executable.clone(),
                is_temporary: false,
            }),
            Self::Portable(firefox) => Ok(PreparedExecutable {
                path: firefox.unique_executable()?,
                is_temporary: true,
            }),
        }
    }
}

/// Looks for an existing Mozilla Firefox installation on the system.
///
/// Checks the `Q_FIREFOX` environment variable, the Windows registry
/// (`App Paths\firefox.exe` in `HKCU` and `HKLM`), and standard installation
/// directories under `ProgramFiles` and `LOCALAPPDATA`.
#[must_use]
pub fn find_system_firefox() -> Option<PathBuf> {
    if let Some(custom) = std::env::var_os("Q_FIREFOX") {
        let path = PathBuf::from(custom);
        if path.is_file() {
            return Some(path);
        }
    }

    #[cfg(target_os = "windows")]
    {
        find_firefox_in_registry().or_else(find_firefox_on_disk)
    }

    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

#[cfg(target_os = "windows")]
fn find_firefox_in_registry() -> Option<PathBuf> {
    const SUBKEYS: &[&str] = &[
        r"Software\Microsoft\Windows\CurrentVersion\App Paths\firefox.exe",
        r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths\firefox.exe",
    ];

    for root in [
        &windows_registry::CURRENT_USER,
        &windows_registry::LOCAL_MACHINE,
    ] {
        for subkey in SUBKEYS {
            let Ok(key) = root.open(subkey) else {
                continue;
            };
            let Ok(raw) = key.get_string("") else {
                continue;
            };
            let cleaned = raw.trim().trim_matches('"');
            let path = PathBuf::from(cleaned);
            if path.is_file() {
                return Some(path);
            }
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn find_firefox_on_disk() -> Option<PathBuf> {
    let mut candidate_dirs = Vec::new();

    if let Some(prog_files) = std::env::var_os("ProgramFiles") {
        candidate_dirs.push(PathBuf::from(prog_files));
    }
    if let Some(prog_files_x86) = std::env::var_os("ProgramFiles(x86)") {
        candidate_dirs.push(PathBuf::from(prog_files_x86));
    }
    if let Some(prog_w6432) = std::env::var_os("ProgramW6432") {
        candidate_dirs.push(PathBuf::from(prog_w6432));
    }
    if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
        candidate_dirs.push(PathBuf::from(local_appdata));
    }

    candidate_dirs.push(PathBuf::from(r"C:\Program Files"));
    candidate_dirs.push(PathBuf::from(r"C:\Program Files (x86)"));

    for dir in candidate_dirs {
        let exe = dir.join("Mozilla Firefox").join("firefox.exe");
        if exe.is_file() {
            return Some(exe);
        }
    }

    None
}

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
        installer::download_installer(&installer)?;
        println!("Checking the browser...");
        installer::verify_checksum(&installer)?;
        println!("Unpacking the browser...");
        installer::unpack(&installer, &firefox.root)?;
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

/// Returns the path of a binary in `%SystemRoot%\System32`.
pub(crate) fn system32_path(binary: &str) -> PathBuf {
    let sys_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    Path::new(&sys_root).join("System32").join(binary)
}

/// Creates a `Command` targeting a binary in `%SystemRoot%\System32`.
pub(crate) fn system32_cmd(binary: &str) -> std::process::Command {
    std::process::Command::new(system32_path(binary))
}

/// Removes a file, logging failures other than "not found".
#[allow(clippy::print_stderr)]
pub(super) fn remove_file_quiet(path: &Path) {
    if let Err(error) = fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            eprintln!("warning: cannot remove {}: {error}", path.display());
        }
    }
}

/// Removes a directory tree, logging failures other than "not found".
#[allow(clippy::print_stderr)]
pub(super) fn remove_dir_quiet(path: &Path) {
    if let Err(error) = fs::remove_dir_all(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            eprintln!("warning: cannot remove {}: {error}", path.display());
        }
    }
}

/// Returns a suffix that no other file of this process uses.
pub(super) fn unique_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());

    format!("{}-{nanos:x}", std::process::id())
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{
        find_system_firefox, unique_suffix, Browser, FIREFOX_SHA256, FIREFOX_URL, FIREFOX_VERSION,
    };
    use std::path::PathBuf;

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
    fn the_suffix_differs_between_two_calls() {
        assert_ne!(unique_suffix(), unique_suffix());
    }

    #[test]
    fn browser_distinguishes_system_and_portable() -> anyhow::Result<()> {
        let system_path = PathBuf::from(r"C:\Program Files\Mozilla Firefox\firefox.exe");
        let browser = Browser::System {
            executable: system_path.clone(),
        };
        assert!(!browser.is_portable());
        assert_eq!(browser.launch_executable()?, system_path);
        let prepared = browser.prepare_executable()?;
        assert_eq!(prepared.path, system_path);
        assert!(!prepared.is_temporary);
        Ok(())
    }

    #[test]
    fn finds_system_firefox_respects_env_var() -> anyhow::Result<()> {
        let dummy = std::env::temp_dir().join("fake_firefox.exe");
        std::fs::write(&dummy, b"dummy")?;
        std::env::set_var("Q_FIREFOX", &dummy);
        assert_eq!(find_system_firefox(), Some(dummy.clone()));
        std::env::remove_var("Q_FIREFOX");
        let _ = std::fs::remove_file(&dummy);
        Ok(())
    }
}

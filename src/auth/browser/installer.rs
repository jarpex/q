//! Download, checksum verification, and 7-Zip extraction of the Firefox installer.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use super::{system32_path, FIREFOX_SHA256, FIREFOX_URL};

/// Signature that starts a 7-Zip archive.
const ARCHIVE_SIGNATURE: [u8; 6] = [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];

/// Name of the browser executable inside the unpacked archive.
const EXECUTABLE_NAME: &str = "core/firefox.exe";

/// Downloads the Firefox installer.
///
/// The download runs through the curl.exe that ships with Windows, at its
/// absolute path, so it does not depend on the Python interpreter of the
/// user and cannot be redirected by a hostile executable earlier on PATH.
///
/// # Errors
///
/// Returns an error if curl cannot be run or the download fails.
pub(super) fn download_installer(destination: &Path) -> Result<()> {
    // Use the curl.exe that ships with Windows 10 and later, at its absolute
    // path, so the download does not depend on the Python interpreter of the
    // user and cannot be redirected by a hostile executable earlier on PATH.
    let curl = system32_path("curl.exe");

    let output = std::process::Command::new(&curl)
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
pub(super) fn verify_checksum(path: &Path) -> Result<()> {
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
pub(super) fn unpack(installer: &Path, destination: &Path) -> Result<()> {
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
    !matches!(
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
    )
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

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{is_executable, normalize, safe_relative_path, EXECUTABLE_NAME};
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
    fn the_executable_sits_in_the_core_directory() {
        assert_eq!(EXECUTABLE_NAME, "core/firefox.exe");
    }
}

//! Checks that the script of the release tag keeps the tag and the version
//! together.
//!
//! A tag that goes with another version publishes a binary under a version
//! that the manifest and the man page do not carry, and the mistake shows up
//! only in the release that is already out.

#![allow(clippy::result_large_err)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

/// The root of the repository.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The path of the script.
fn script() -> PathBuf {
    repo_root().join("scripts/check-release-tag.sh")
}

/// A copy of the files that the script reads.
struct Files {
    /// The directory with the copies.
    dir: PathBuf,
    /// The path of the manifest.
    manifest: PathBuf,
    /// The path of the man page.
    man_page: PathBuf,
}

impl Files {
    /// Copies the manifest and the man page of the repository.
    ///
    /// The name keeps the directories of the tests apart, for they run at once.
    fn copy(name: &str) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!("q-release-tag-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("man"))?;

        let manifest = dir.join("Cargo.toml");
        let man_page = dir.join("man/q.1");
        fs::copy(repo_root().join("Cargo.toml"), &manifest)?;
        fs::copy(repo_root().join("man/q.1"), &man_page)?;

        Ok(Self {
            dir,
            manifest,
            man_page,
        })
    }

    /// Runs the script with the copies.
    fn run(&self, tag: &str) -> Result<bool> {
        let output = Command::new("bash")
            .arg(script())
            .arg(tag)
            .env("Q_MANIFEST", &self.manifest)
            .env("Q_MAN_PAGE", &self.man_page)
            .output()
            .context("Cannot run the script of the release tag")?;
        Ok(output.status.success())
    }

    /// Changes the version of the man page.
    fn set_man_version(&self, version: &str) -> Result<()> {
        let page = fs::read_to_string(&self.man_page)?;
        let replaced = page.replace("\"q 0.4.0\"", &format!("\"q {version}\""));
        fs::write(&self.man_page, replaced)?;
        Ok(())
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn the_script_accepts_the_version_of_the_manifest() -> Result<()> {
    let files = Files::copy("accept")?;
    let version = current_version();
    anyhow::ensure!(
        files.run(&format!("v{version}"))?,
        "the script refuses the tag of the current version"
    );
    Ok(())
}

#[test]
fn the_script_refuses_another_version() -> Result<()> {
    let files = Files::copy("other")?;
    anyhow::ensure!(
        !files.run("v99.99.99")?,
        "the script accepts a tag that goes with another version"
    );
    Ok(())
}

#[test]
fn the_script_refuses_a_broken_tag() -> Result<()> {
    let files = Files::copy("broken")?;
    for tag in ["0.4.0", "v0.4", "v0.4.0.1", "release", ""] {
        anyhow::ensure!(!files.run(tag)?, "the script accepts the broken tag {tag}");
    }
    Ok(())
}

#[test]
fn the_script_accepts_a_release_without_a_man_page() -> Result<()> {
    let files = Files::copy("no-man")?;
    fs::remove_file(&files.man_page)?;
    let version = current_version();
    anyhow::ensure!(
        files.run(&format!("v{version}"))?,
        "the script refuses a release that carries no man page"
    );
    Ok(())
}

#[test]
fn the_script_refuses_a_man_page_of_another_version() -> Result<()> {
    let files = Files::copy("man")?;
    files.set_man_version("0.3.0")?;
    let version = current_version();
    anyhow::ensure!(
        !files.run(&format!("v{version}"))?,
        "the script accepts a man page of another version"
    );
    Ok(())
}

/// The version that the manifest of the repository carries.
fn current_version() -> String {
    let text = fs::read_to_string(repo_root().join("Cargo.toml")).unwrap_or_default();
    for line in text.lines() {
        let Some(tail) = line.strip_prefix("version") else {
            continue;
        };
        let trimmed = tail.trim_start();
        let after = trimmed.strip_prefix('=').unwrap_or(trimmed);
        let version = after.trim().trim_matches('"');
        if !version.is_empty() {
            return version.to_owned();
        }
    }
    String::new()
}

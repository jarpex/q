//! Checks that the release, the formula and the workflow of the formula agree.
//!
//! The workflow of the formula rewrites the downloads of the formula with the
//! assets of the release. A name that the release does not carry, or a
//! download that the workflow does not fetch, breaks the release of the next
//! version, and the break shows up only in the run of the workflow.

#![allow(clippy::result_large_err)]

use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, Result};

/// The root of the repository.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Reads a file of the repository.
///
/// # Errors
/// Returns an error when the file is missing or unreadable.
fn read_repo_file(name: &str) -> Result<String> {
    fs::read_to_string(repo_root().join(name))
        .map_err(|error| anyhow!("cannot read {name}: {error}"))
}

/// Takes the text between the quotes of a line that starts with a prefix.
///
/// The line is trimmed first, so the indentation of the file does not matter.
fn quoted<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let tail = line.trim().strip_prefix(prefix)?;
    let body = tail.strip_prefix('"')?;
    body.strip_suffix('"')
}

/// One entry of the matrix of the release.
struct Asset {
    /// The name of the archive, without the extension.
    name: String,
    /// The kind of the archive.
    archive: String,
}

/// The assets that the matrix of the release builds.
///
/// # Errors
/// Returns an error when the workflow is missing or unreadable.
fn release_assets() -> Result<Vec<Asset>> {
    let workflow = read_repo_file(".github/workflows/release.yml")?;
    let mut assets: Vec<Asset> = Vec::new();
    let mut current: Option<Asset> = None;

    for line in workflow.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("- os:") {
            if let Some(asset) = current.take() {
                assets.push(asset);
            }
            current = Some(Asset {
                name: String::new(),
                archive: String::new(),
            });
            continue;
        }
        let Some(asset) = current.as_mut() else {
            continue;
        };
        if let Some(value) = trimmed.strip_prefix("asset_name:") {
            value.trim().clone_into(&mut asset.name);
        } else if let Some(value) = trimmed.strip_prefix("archive:") {
            value.trim().clone_into(&mut asset.archive);
        }
    }
    if let Some(asset) = current.take() {
        assets.push(asset);
    }
    Ok(assets)
}

/// The names of the assets that the workflow of the formula fetches.
///
/// # Errors
/// Returns an error when the workflow is missing or unreadable.
fn formula_assets() -> Result<Vec<String>> {
    let workflow = read_repo_file(".github/workflows/update-formula.yml")?;
    let mut names: Vec<String> = Vec::new();
    for line in workflow.lines() {
        let Some(list) = quoted(line, "NAMES=") else {
            continue;
        };
        for name in list.split_whitespace() {
            if !names.iter().any(|known| known == name) {
                names.push(name.to_owned());
            }
        }
    }
    Ok(names)
}

/// The names of the downloads that the formula asks for.
///
/// # Errors
/// Returns an error when the formula is missing or unreadable.
fn formula_downloads() -> Result<Vec<String>> {
    let formula = read_repo_file("tests/fixtures/q.rb")?;
    let mut names: Vec<String> = Vec::new();
    for line in formula.lines() {
        let Some(link) = quoted(line, "url ") else {
            continue;
        };
        let Some(file) = link.rsplit('/').next() else {
            continue;
        };
        let asset = file.strip_suffix(".tar.gz").unwrap_or(file);
        if !names.iter().any(|known| known == asset) {
            names.push(asset.to_owned());
        }
    }
    Ok(names)
}

/// The targets that the matrix of the release builds for Windows.
///
/// # Errors
/// Returns an error when the workflow is missing or unreadable.
fn release_targets() -> Result<Vec<String>> {
    let workflow = read_repo_file(".github/workflows/release.yml")?;
    let mut targets: Vec<String> = Vec::new();
    for line in workflow.lines() {
        let Some(rest) = line.trim().strip_prefix("target:") else {
            continue;
        };
        let target = rest.trim();
        if target.contains("windows") && !targets.iter().any(|known| known == target) {
            targets.push(target.to_owned());
        }
    }
    Ok(targets)
}

#[test]
fn the_release_builds_every_windows_target() -> Result<()> {
    let targets = release_targets()?;
    for expected in [
        "i686-pc-windows-msvc",
        "x86_64-pc-windows-msvc",
        "aarch64-pc-windows-msvc",
    ] {
        assert!(
            targets.iter().any(|known| known == expected),
            "the release does not build {expected}"
        );
    }
    Ok(())
}

#[test]
fn the_workflow_of_the_formula_fetches_assets_of_the_release() -> Result<()> {
    let assets = release_assets()?;
    for name in formula_assets()? {
        let found = assets.iter().any(|asset| {
            asset.name == name && (asset.archive.is_empty() || asset.archive == "tar.gz")
        });
        assert!(
            found,
            "the workflow of the formula fetches {name}, which the release does not build"
        );
    }
    Ok(())
}

#[test]
fn the_formula_asks_only_for_assets_of_the_workflow() -> Result<()> {
    let names = formula_assets()?;
    for download in formula_downloads()? {
        assert!(
            names.contains(&download),
            "the formula asks for {download}, which the workflow of the formula does not fetch"
        );
    }
    Ok(())
}

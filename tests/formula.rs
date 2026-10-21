//! Checks that the script of the formula keeps the formula in shape.
//!
//! The script runs in the workflow of the release, so a break in it would
//! leave the formula of an old version behind without a sign.

#![allow(clippy::result_large_err)]

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;

/// The path of the script.
fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/update-formula.py")
}

/// The path of the fixture that holds the shape of the formula.
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/q.rb")
}

/// A download of the formula and its hash.
type Download = (&'static str, String);

/// The hashes of the downloads, one for every platform.
fn hashes() -> Vec<Download> {
    vec![
        ("q-macos-apple-silicon.tar.gz", "a".repeat(64)),
        ("q-macos-intel.tar.gz", "b".repeat(64)),
        ("q-linux-arm64.tar.gz", "c".repeat(64)),
        ("q-linux-amd64.tar.gz", "d".repeat(64)),
    ]
}

/// The name of the python of the system, when there is one.
fn python() -> Option<&'static str> {
    for name in ["python3", "python"] {
        let Ok(output) = Command::new(name).arg("--version").output() else {
            continue;
        };
        if output.status.success() {
            return Some(name);
        }
    }
    None
}

/// Runs the script on a copy of the fixture and returns the result.
///
/// The name keeps the directories of the tests apart, for they run at once.
fn update(version: &str, name: &str) -> Result<String> {
    let Some(python) = python() else {
        return Ok(String::new());
    };
    let dir = std::env::temp_dir().join(format!("q-formula-{name}-{version}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let target = dir.join("q.rb");
    std::fs::copy(fixture(), &target)?;

    let mut args = vec![
        script().to_string_lossy().into_owned(),
        target.to_string_lossy().into_owned(),
        version.to_owned(),
    ];
    for (asset, digest) in hashes() {
        args.push(format!("{asset}={digest}"));
    }

    let output = Command::new(python)
        .args(&args)
        .output()
        .context("Cannot run the script of the formula")?;
    anyhow::ensure!(
        output.status.success(),
        "the script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let text = std::fs::read_to_string(&target)?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(text)
}

#[test]
fn the_script_raises_the_version_and_the_downloads() -> Result<()> {
    if python().is_none() {
        return Ok(());
    }
    let text = update("0.5.0", "version")?;

    anyhow::ensure!(text.contains("version \"0.5.0\""), "{text}");
    anyhow::ensure!(!text.contains("version \"0.4.0\""), "{text}");
    for (asset, digest) in hashes() {
        anyhow::ensure!(text.contains(&digest), "the hash of {asset} is missing");
        anyhow::ensure!(text.contains(asset), "the download of {asset} is missing");
    }
    anyhow::ensure!(
        text.contains("/v0.5.0/"),
        "the downloads do not carry the new version: {text}"
    );
    Ok(())
}

#[test]
fn the_script_keeps_the_head_and_the_license() -> Result<()> {
    if python().is_none() {
        return Ok(());
    }
    let text = update("0.5.0", "head")?;

    anyhow::ensure!(text.contains("license \"MIT\""), "{text}");
    anyhow::ensure!(
        text.contains("head \""),
        "the head of the formula is gone: {text}"
    );
    anyhow::ensure!(
        text.contains("homepage \""),
        "the page of the formula is gone: {text}"
    );
    Ok(())
}

#[test]
fn the_script_is_idempotent() -> Result<()> {
    if python().is_none() {
        return Ok(());
    }
    let first = update("0.5.0", "first")?;
    let second = update("0.5.0", "second")?;

    anyhow::ensure!(first == second, "the script changed the formula twice");
    Ok(())
}

#[test]
fn the_script_fails_when_a_hash_is_missing() -> Result<()> {
    let Some(python) = python() else {
        return Ok(());
    };
    let dir = std::env::temp_dir().join("q-formula-missing-hash");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let target = dir.join("q.rb");
    std::fs::copy(fixture(), &target)?;

    let output = Command::new(python)
        .arg(script())
        .arg(&target)
        .arg("0.5.0")
        .arg("q-macos-intel.tar.gz=abc")
        .output()
        .context("Cannot run the script of the formula")?;

    anyhow::ensure!(
        !output.status.success(),
        "the script accepted a formula without every hash"
    );
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn the_fixture_has_the_shape_of_a_formula() -> Result<()> {
    let text = std::fs::read_to_string(fixture())?;
    anyhow::ensure!(text.contains("class Q < Formula"), "{text}");
    anyhow::ensure!(text.contains("on_macos do"), "{text}");
    anyhow::ensure!(text.contains("on_linux do"), "{text}");
    Ok(())
}

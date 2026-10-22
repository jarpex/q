//! Checks that the build of the release runs only for a tag.
//!
//! A release has two paths: an ordinary push runs the gates of the CI, and a
//! push of a tag runs the gates, the build of the binaries, the notes of the
//! release and the update of the formula. A build that runs for an ordinary
//! push spends the runners of the CI for a binary that nobody waits for.

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

/// The lines of a workflow that start with a key of the trigger.
///
/// The keys of the trigger sit at the start of a line, while the keys of the
/// jobs sit deeper, so the indentation tells them apart.
fn trigger_lines(workflow: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_trigger = false;
    for line in workflow.lines() {
        if line.starts_with("on:") {
            in_trigger = true;
            continue;
        }
        if !in_trigger {
            continue;
        }
        if line.is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            in_trigger = false;
            continue;
        }
        lines.push(line.trim().to_owned());
    }
    lines
}

#[test]
fn the_release_runs_only_for_a_tag() -> Result<()> {
    let workflow = read_repo_file(".github/workflows/release.yml")?;
    let trigger = trigger_lines(&workflow);

    assert!(
        trigger.iter().any(|line| line.starts_with("tags:")),
        "the release workflow does not watch the tags"
    );
    assert!(
        !trigger.iter().any(|line| line.starts_with("branches:")),
        "the release workflow runs for a push of a branch"
    );
    Ok(())
}

#[test]
fn the_ci_builds_nothing() -> Result<()> {
    let workflow = read_repo_file(".github/workflows/ci.yml")?;
    assert!(
        !workflow.contains("cargo build --release"),
        "the CI of an ordinary push builds a binary"
    );
    assert!(
        !workflow.contains("package-asset.sh"),
        "the CI of an ordinary push packs a release asset"
    );
    Ok(())
}

#[test]
fn the_release_builds_and_updates_the_formula() -> Result<()> {
    let workflow = read_repo_file(".github/workflows/release.yml")?;
    for step in [
        "cargo build --release --locked --target",
        "package-asset.sh",
        "git-cliff --current",
        "Update the Homebrew Formula",
    ] {
        assert!(
            workflow.contains(step),
            "the release workflow does not run {step}"
        );
    }
    Ok(())
}

#[test]
fn the_release_does_not_overwrite_a_published_release() -> Result<()> {
    let workflow = read_repo_file(".github/workflows/release.yml")?;
    assert!(
        workflow.contains("gh release view"),
        "the release workflow does not check whether the release is out yet"
    );
    Ok(())
}

#[test]
fn the_formula_waits_for_the_assets_of_the_release() -> Result<()> {
    let workflow = read_repo_file(".github/workflows/update-formula.yml")?;
    let trigger = trigger_lines(&workflow);

    assert!(
        !trigger.iter().any(|line| line.starts_with("release:")),
        "the formula is updated from the event of the release, which fires before the assets are there"
    );
    assert!(
        workflow.contains("workflow_call:"),
        "the formula is not updated from the release workflow"
    );
    Ok(())
}

#[test]
fn the_release_checks_the_tag_against_the_version() -> Result<()> {
    let workflow = read_repo_file(".github/workflows/release.yml")?;
    assert!(
        workflow.contains("check-release-tag.sh"),
        "the release workflow does not check the tag against the version"
    );
    Ok(())
}

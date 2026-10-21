//! Checks that every fuzz target has a recipe in the justfile.
//!
//! A target without a recipe never runs, so the coverage of the fuzz suite
//! would drift away from the targets that exist.

#![allow(clippy::result_large_err)]

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

/// The root of the repository.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Reads a file of the repository.
///
/// # Errors
/// Returns an error when the file is missing or unreadable.
fn read_repo_file(name: &str) -> Result<String> {
    fs::read_to_string(repo_root().join(name))
        .map_err(|error| anyhow!("cannot read {name}: {error}"))
}

/// The names of the fuzz targets that the manifest declares.
///
/// # Errors
/// Returns an error when the manifest is missing or unreadable.
fn declared_targets() -> Result<Vec<String>> {
    let manifest = read_repo_file("fuzz/Cargo.toml")?;
    let mut targets = Vec::new();
    let mut lines = manifest.lines();
    while let Some(line) = lines.next() {
        if line.trim() != "[[bin]]" {
            continue;
        }
        for next in lines.by_ref() {
            let trimmed = next.trim();
            if let Some(name) = trimmed.strip_prefix("name = \"") {
                targets.push(name.trim_end_matches('"').to_owned());
                break;
            }
            if trimmed.starts_with('[') {
                break;
            }
        }
    }
    Ok(targets)
}

/// The names of the targets that the recipes of the justfile run.
///
/// # Errors
/// Returns an error when the justfile is missing or unreadable.
fn covered_targets() -> Result<Vec<String>> {
    let justfile = read_repo_file("justfile")?;
    let mut targets = Vec::new();
    for line in justfile.lines() {
        if let Some(rest) = line.trim().strip_prefix("just _fuzz-target ") {
            targets.push(
                rest.split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
            );
        }
    }
    Ok(targets)
}

/// The body of the recipe that runs every target.
///
/// # Errors
/// Returns an error when the justfile is missing or unreadable.
fn full_suite() -> Result<String> {
    let justfile = read_repo_file("justfile")?;
    Ok(justfile
        .split("fuzz-all")
        .nth(1)
        .unwrap_or_default()
        .lines()
        .take_while(|line| line.trim().is_empty() || line.starts_with(' '))
        .collect::<Vec<_>>()
        .join("\n"))
}

#[test]
fn every_fuzz_target_has_a_recipe() -> Result<()> {
    let declared = declared_targets()?;
    assert!(!declared.is_empty(), "no fuzz targets found");

    let covered = covered_targets()?;
    for target in &declared {
        assert!(
            covered.contains(target),
            "the target {target} has no recipe in the justfile"
        );
    }
    Ok(())
}

#[test]
fn every_recipe_runs_an_existing_target() -> Result<()> {
    let declared = declared_targets()?;
    for target in covered_targets()? {
        assert!(
            declared.contains(&target),
            "the recipe runs the unknown target {target}"
        );
    }
    Ok(())
}

#[test]
fn every_fuzz_target_runs_in_the_full_suite() -> Result<()> {
    let suite = full_suite()?;
    for target in declared_targets()? {
        let recipe = format!("fuzz-{}", target.replace('_', "-"));
        assert!(
            suite.contains(&recipe),
            "the target {target} does not run in fuzz-all"
        );
    }
    Ok(())
}

//! Checks that the man page holds what the command line offers.
//!
//! The page is written by hand, so it drifts away from the options of the
//! tool. These tests catch the drift.

#![allow(clippy::result_large_err)]

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// The path of the man page.
fn man_page() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("man/q.1")
}

/// The text of the man page.
fn page() -> Result<String> {
    let path = man_page();
    fs::read_to_string(&path)
        .with_context(|| format!("Cannot read the man page: {}", path.display()))
}

/// The options that the man page declares in its section of the options.
///
/// Only the lines that name an option count, so the text around them cannot
/// hide a missing option.
fn declared_options() -> Result<Vec<String>> {
    let mut names = Vec::new();
    let mut in_options = false;
    // A declaration of an option follows a tag paragraph.
    let mut after_tag = false;

    for line in page()?.lines() {
        if line.starts_with(".SH OPTIONS") {
            in_options = true;
            continue;
        }
        if in_options && line.starts_with(".SH ") {
            break;
        }
        if !in_options {
            continue;
        }
        if line.starts_with(".TP") {
            after_tag = true;
            continue;
        }
        if !(line.starts_with(".B ") || line.starts_with(".BR ")) {
            continue;
        }
        // The text of an option may name a flag of another program, so only
        // the line that follows the tag counts.
        if !after_tag {
            continue;
        }
        after_tag = false;

        for word in line.split_whitespace() {
            let unescaped = word.replace("\\-", "-");
            let token = unescaped.trim_matches(|character| {
                character == '"' || character == ',' || character == '(' || character == ')'
            });
            // The long option has two dashes, the short one has one.
            let candidate = token.strip_prefix("--").or_else(|| token.strip_prefix('-'));
            let Some(option) = candidate else {
                continue;
            };
            if option.is_empty() || option.contains('\\') {
                continue;
            }
            names.push(option.to_owned());
        }
    }
    Ok(names)
}

/// The options of the command line, as the help of the tool prints them.
fn options() -> Result<Vec<String>> {
    let output = Command::new(env!("CARGO_BIN_EXE_q"))
        .arg("--help")
        .output()
        .context("Cannot run q --help")?;
    anyhow::ensure!(output.status.success(), "q --help failed");

    let help = String::from_utf8_lossy(&output.stdout);
    let mut names = Vec::new();
    for line in help.lines() {
        let trimmed = line.trim();
        // The options are the lines that start with a dash.
        if !trimmed.starts_with('-') {
            continue;
        }
        for part in trimmed.split(", ") {
            let name = part.trim();
            if let Some(long) = name.strip_prefix("--") {
                names.push(long.split(' ').next().unwrap_or(long).to_owned());
            } else if let Some(short) = name.strip_prefix('-') {
                names.push(short.to_owned());
            }
        }
    }
    Ok(names)
}

#[test]
fn the_man_page_holds_every_option() -> Result<()> {
    let declared = declared_options()?;

    for option in options()? {
        if option == "help" || option == "version" {
            continue;
        }
        anyhow::ensure!(
            declared.iter().any(|name| name == &option),
            "the man page does not declare the option --{option}: {declared:?}"
        );
    }
    Ok(())
}

#[test]
fn the_man_page_declares_nothing_else() -> Result<()> {
    let declared = declared_options()?;
    let known = options()?;

    for name in &declared {
        anyhow::ensure!(
            known.iter().any(|option| option == name),
            "the man page declares the option --{name}, which the tool does not have"
        );
    }
    Ok(())
}

#[test]
fn the_man_page_holds_the_sections() -> Result<()> {
    let page = page()?;

    for section in [
        "NAME",
        "SYNOPSIS",
        "DESCRIPTION",
        "OPTIONS",
        "EXAMPLES",
        "CONFIGURATION",
        "FILES",
        "ENVIRONMENT",
        "SEE ALSO",
        "LICENSE",
    ] {
        let plain = format!(".SH {section}");
        let quoted = format!(".SH \"{section}\"");
        anyhow::ensure!(
            page.contains(&plain) || page.contains(&quoted),
            "the man page does not hold the section {section}"
        );
    }
    Ok(())
}

#[test]
fn the_man_page_holds_the_version_of_the_tool() -> Result<()> {
    let output = Command::new(env!("CARGO_BIN_EXE_q"))
        .arg("--version")
        .output()
        .context("Cannot run q --version")?;
    let printed = String::from_utf8_lossy(&output.stdout);
    let version = printed.trim().strip_prefix("q ").unwrap_or("").to_owned();

    anyhow::ensure!(!version.is_empty(), "the version is empty");
    anyhow::ensure!(
        page()?.contains(&format!("\"q {version}\"")),
        "the man page does not hold the version {version}"
    );
    Ok(())
}

#[test]
fn the_man_page_holds_the_options_of_the_config_file() -> Result<()> {
    let page = page()?;

    for option in [
        "model",
        "no\\-stream",
        "auth\\-timeout",
        "provider",
        "uc",
        "uc\\-path",
    ] {
        anyhow::ensure!(
            page.contains(option),
            "the man page does not hold the option {option} of the config file"
        );
    }
    Ok(())
}

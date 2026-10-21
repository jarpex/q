//! Runs the q binary through a full session against a stubbed Gemini API.

#![allow(clippy::result_large_err)]

mod support;

use std::process::Command;
use support::{TempDir, TestEnv};

/// Prepares the environment, or skips the test when no Python is available.
fn env() -> Option<TestEnv> {
    TestEnv::new().ok().flatten()
}

/// Runs the binary in a directory that has no metadata yet.
///
/// The venv path is a file, so the setup fails before it reaches the network,
/// but the startup messages are already printed by then.
fn run_on_a_first_run() -> anyhow::Result<String> {
    let dir = TempDir::new("first-run")?;
    std::fs::write(dir.path().join("venv"), b"not a directory")?;

    let output = Command::new(env!("CARGO_BIN_EXE_q"))
        .arg("hello")
        .env("Q_CONFIG_DIR", dir.path())
        .env("Q_DISABLE_CLIPBOARD", "1")
        .output()?;

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[test]
fn announces_the_first_run_without_a_version_change() -> anyhow::Result<()> {
    let stdout = run_on_a_first_run()?;

    assert!(stdout.contains("First run"), "stdout: {stdout}");
    assert!(!stdout.contains("Version changed"), "stdout: {stdout}");
    assert!(!stdout.contains("hours old"), "stdout: {stdout}");
    Ok(())
}

#[test]
fn answers_a_query_in_chat_mode() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };
    env.set_response("four")?;

    let output = env.run(&["what is two plus two"])?;

    assert!(output.status.success(), "q failed: {output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "four\n");

    // The request must reach the API with the query and the model.
    let log = env.log();
    assert!(log.contains("generate_content_stream"), "log: {log}");
    assert!(log.contains("what is two plus two"), "log: {log}");
    Ok(())
}

#[test]
fn answers_a_query_in_batch_mode() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };
    env.set_response("four")?;

    let output = env.run(&["--no-stream", "what is two plus two"])?;

    assert!(output.status.success(), "q failed: {output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "four\n");

    let log = env.log();
    assert!(log.contains("generate_content"), "log: {log}");
    assert!(!log.contains("generate_content_stream"), "log: {log}");
    Ok(())
}

#[test]
fn generates_a_command_in_command_mode() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };
    env.set_response("ls -la")?;

    let output = env.run(&["--command-mode", "list the files"])?;

    assert!(output.status.success(), "q failed: {output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "$ ls -la\n");

    let log = env.log();
    assert!(log.contains("command generator"), "log: {log}");
    Ok(())
}

#[test]
fn lists_the_available_models() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };

    let output = env.run(&["--list-models"])?;

    assert!(output.status.success(), "q failed: {output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "Available models: Gemini 2.5 Pro: gemini-2.5-pro\n"
    );
    Ok(())
}

#[test]
fn shows_the_usage_limits() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };

    let output = env.run(&["--usage-limits"])?;

    assert!(output.status.success(), "q failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage limits:"), "stdout: {stdout}");
    Ok(())
}

#[test]
fn reports_an_empty_response() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };
    env.set_response("")?;

    let output = env.run(&["say nothing"])?;

    assert!(!output.status.success(), "q must fail: {output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("empty response"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn reads_the_model_from_the_configuration_file() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };
    env.set_response("ok")?;
    env.write_config("model=gemini-2.5-pro")?;

    let output = env.run(&["hello"])?;

    assert!(output.status.success(), "q failed: {output:?}");

    // The model travels to the API as an argument.
    let log = env.log();
    assert!(log.contains("gemini-2.5-pro"), "log: {log}");
    Ok(())
}

#[test]
fn reports_an_invalid_configuration_file() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };
    env.write_config("nonsense=1")?;

    let output = env.run(&["hello"])?;

    assert!(!output.status.success(), "q must fail: {output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown configuration option"),
        "stderr: {stderr}"
    );
    Ok(())
}

#[test]
fn requires_a_query() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };

    let output = env.run(&[])?;

    assert!(!output.status.success(), "q must fail: {output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("required arguments were not provided"),
        "stderr: {stderr}"
    );
    Ok(())
}

#[test]
fn imports_the_cookies_from_stdin() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };

    let output = env.run_with_stdin(&["--import-cookies"], "psid-value\npsidts-value\n")?;

    assert!(output.status.success(), "q failed: {output:?}");

    // The imported cookies must reach the disk.
    let cookies = std::fs::read_to_string(env.config_dir().join("cookies.json"))?;
    assert!(cookies.contains("psid-value"), "cookies: {cookies}");
    assert!(cookies.contains("psidts-value"), "cookies: {cookies}");
    Ok(())
}

#[test]
fn reuses_the_stored_cookies() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };
    env.set_response("four")?;

    let output = env.run(&["two plus two"])?;

    assert!(output.status.success(), "q failed: {output:?}");

    // The stored cookies travel to the API through the environment.
    let log = env.log();
    assert!(log.contains("test-psid"), "log: {log}");
    assert!(log.contains("test-psidts"), "log: {log}");
    Ok(())
}

#[test]
fn cleans_the_output_with_the_uc_program() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };
    env.set_response("four\u{a0}five")?;
    let uc = env.write_uc_stub()?;

    let output = env.run(&[
        "--uc",
        "-c -e cp1251",
        "--uc-path",
        uc.to_str().unwrap_or_default(),
        "two plus two",
    ])?;

    assert!(output.status.success(), "q failed: {output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "fourfive\n");
    Ok(())
}

#[test]
fn cleans_the_output_in_batch_mode() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };
    env.set_response("four\u{a0}five")?;
    let uc = env.write_uc_stub()?;

    let output = env.run(&[
        "--no-stream",
        "--uc",
        "-c -e cp1251",
        "--uc-path",
        uc.to_str().unwrap_or_default(),
        "two plus two",
    ])?;

    assert!(output.status.success(), "q failed: {output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "fourfive\n");
    Ok(())
}

#[test]
fn cleans_the_command_in_command_mode() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };
    env.set_response("ls -la")?;
    let uc = env.write_uc_stub()?;

    let output = env.run(&[
        "--command-mode",
        "--uc",
        "-c -e cp1251",
        "--uc-path",
        uc.to_str().unwrap_or_default(),
        "list the files",
    ])?;

    assert!(output.status.success(), "q failed: {output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "$ ls -la\n");
    Ok(())
}

#[test]
fn the_uc_program_always_clears_the_text() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };
    env.set_response("four")?;
    let uc = env.write_uc_stub()?;

    let output = env.run(&[
        "--uc",
        "-s -c -e cp1251",
        "--uc-path",
        uc.to_str().unwrap_or_default(),
        "two plus two",
    ])?;

    assert!(output.status.success(), "q failed: {output:?}");

    // The show flag never reaches the program, and the clear flag comes first.
    let args = env.uc_args();
    assert_eq!(args, "-c\n-e\ncp1251");
    Ok(())
}

#[test]
fn the_uc_options_come_from_the_configuration_file() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };
    env.set_response("four\u{a0}five")?;
    let uc = env.write_uc_stub()?;
    env.write_config(&format!(
        "uc = -c -e cp1251\nuc-path = {}\n",
        uc.to_str().unwrap_or_default()
    ))?;

    let output = env.run(&["two plus two"])?;

    assert!(output.status.success(), "q failed: {output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "fourfive\n");
    Ok(())
}

#[test]
fn warns_when_the_uc_program_is_missing() -> anyhow::Result<()> {
    let Some(env) = env() else {
        return Ok(());
    };
    env.set_response("four")?;

    let output = env.run(&["--uc", "-c", "--uc-path", "/nonexistent/uc", "two plus two"])?;

    // The program does not fall over, it prints the text as it is.
    assert!(output.status.success(), "q failed: {output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "four\n");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("uc not found"), "stderr: {stderr}");
    Ok(())
}

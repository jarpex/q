//! Checks that the command-line timeout wins over the configuration file value.

#![allow(clippy::result_large_err)]

use clap::Parser;
use q::cli::Cli;
use q::config::settings::Settings;

fn resolve(cli_args: &[&str], settings: &Settings) -> u64 {
    let result = Cli::try_parse_from(cli_args);
    assert!(result.is_ok());
    let Ok(mut cli) = result else {
        return 0;
    };
    if cli.auth_timeout.is_none() {
        cli.auth_timeout = settings.auth_timeout;
    }
    cli.effective_auth_timeout().as_secs()
}

#[test]
fn command_line_wins_over_config() {
    let settings = Settings {
        auth_timeout: Some(777),
        ..Default::default()
    };
    assert_eq!(
        resolve(&["q", "--auth-timeout", "42", "query"], &settings),
        42
    );
}

#[test]
fn config_used_when_no_flag() {
    let settings = Settings {
        auth_timeout: Some(777),
        ..Default::default()
    };
    assert_eq!(resolve(&["q", "query"], &settings), 777);
}

#[test]
fn default_used_when_neither_sets_it() {
    let settings = Settings::default();
    assert_eq!(resolve(&["q", "query"], &settings), 300);
}

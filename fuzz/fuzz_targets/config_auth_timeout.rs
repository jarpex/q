#![no_main]
use clap::Parser;
use libfuzzer_sys::fuzz_target;
use q::cli::Cli;
use q::config::settings::{
    parse_auth_timeout, parse_settings, DEFAULT_AUTH_TIMEOUT_SECS, MAX_AUTH_TIMEOUT_SECS,
};

fuzz_target!(|data: &str| {
    let parsed = parse_auth_timeout(data).ok();

    if let Some(seconds) = parsed {
        assert!(seconds >= 1, "timeout must be at least one second");
        assert!(seconds <= MAX_AUTH_TIMEOUT_SECS, "timeout must not exceed the maximum");
    }

    // The command line passes the value through unchanged, so both must agree.
    let from_cli = Cli::try_parse_from(["q", "--auth-timeout", data, "query"]);
    assert_eq!(from_cli.is_ok(), parsed.is_some(), "CLI and parser disagree");

    if let Ok(cli) = from_cli {
        let timeout = cli.effective_auth_timeout().as_secs();
        assert!(timeout >= 1, "timeout must be at least one second");
        assert!(timeout <= MAX_AUTH_TIMEOUT_SECS, "timeout must not exceed the maximum");
    }

    // The config file splits the input into tokens, so the extracted value can
    // differ from the raw input. The extracted value must still be valid.
    if let Some(settings) = parse_settings(&format!("auth-timeout={data}")).ok() {
        if let Some(seconds) = settings.auth_timeout {
            assert!(seconds >= 1, "timeout must be at least one second");
            assert!(seconds <= MAX_AUTH_TIMEOUT_SECS, "timeout must not exceed the maximum");
            let round_trip = parse_auth_timeout(&seconds.to_string());
            assert!(round_trip.is_ok(), "extracted value must parse");
            assert_eq!(round_trip.ok(), Some(seconds));
        }
    }

    // A missing value must fall back to the default.
    let fallback = Cli::try_parse_from(["q", "query"]);
    if let Ok(cli) = fallback {
        assert_eq!(
            cli.effective_auth_timeout().as_secs(),
            DEFAULT_AUTH_TIMEOUT_SECS
        );
    }
});

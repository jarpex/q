use clap::Parser;

use crate::config::settings::{parse_auth_timeout, DEFAULT_AUTH_TIMEOUT_SECS};
use std::path::PathBuf;
use std::time::Duration;

/// Command-line arguments for the q tool.
#[derive(Parser)]
#[command(
    name = env!("CARGO_PKG_NAME"),
    version = env!("CARGO_PKG_VERSION"),
    about = env!("CARGO_PKG_DESCRIPTION"),
    author = env!("CARGO_PKG_AUTHORS")
)]
#[allow(clippy::struct_excessive_bools)]
pub struct Cli {
    /// Run in shell command generation mode instead of chat mode.
    #[arg(short, long)]
    pub command_mode: bool,

    /// Remove saved cookies and authenticate with Gemini again.
    #[arg(long)]
    pub reauth: bool,

    /// List the models available for the authenticated account.
    #[arg(short = 'l', long)]
    pub list_models: bool,

    /// Show Gemini usage limits.
    #[arg(short = 'u', long)]
    pub usage_limits: bool,

    /// Manually import cookies from your browser (recommended for Windows).
    #[arg(short = 'i', long)]
    pub import_cookies: bool,

    /// The Gemini model to use for the query.
    #[arg(short, long)]
    pub model: Option<String>,

    /// Disable streaming and wait for the full response before printing.
    #[arg(long)]
    pub no_stream: bool,

    /// Enable debug output for troubleshooting.
    #[arg(short, long)]
    pub debug: bool,

    /// Force recreation of the Python virtual environment.
    #[arg(long)]
    pub rebuild_venv: bool,

    /// Seconds to wait for the login window before giving up.
    #[arg(long, value_parser = parse_auth_timeout)]
    pub auth_timeout: Option<u64>,

    /// Run the output through the uc program with the given arguments.
    #[arg(long, value_name = "ARGS", allow_hyphen_values = true, num_args = 1)]
    pub uc: Option<String>,

    /// Path of the uc program to run.
    #[arg(long, value_name = "PATH")]
    pub uc_path: Option<PathBuf>,

    /// The provider of the models to use.
    ///
    /// The default provider works through the cookies of the account. Every
    /// other provider needs a token, which the user adds with the flag
    /// `--api-management`.
    #[arg(long, value_name = "PROVIDER")]
    pub provider: Option<String>,

    /// Open the screen that manages the API keys.
    #[arg(long)]
    pub api_management: bool,

    /// The query text to send to the model.
    #[arg(
        required_unless_present_any = [
            "reauth",
            "list_models",
            "usage_limits",
            "rebuild_venv",
            "import_cookies",
            "api_management",
            "help"
        ]
    )]
    pub query: Vec<String>,
}

impl Cli {
    /// Returns the configured model or the default automatic model.
    pub fn effective_model(&self) -> &str {
        self.model.as_deref().unwrap_or("gemini-auto")
    }

    /// Returns the provider to use.
    ///
    /// The command-line value wins over the configuration file value. The
    /// default provider works through the cookies of the account.
    pub fn effective_provider(&self) -> &str {
        self.provider
            .as_deref()
            .unwrap_or(crate::provider::GOOGLE_COOKIES)
    }

    /// Returns the authentication timeout in seconds.
    ///
    /// The command-line value wins over the configuration file value. The
    /// default applies when neither sets one.
    pub fn effective_auth_timeout(&self) -> Duration {
        Duration::from_secs(self.auth_timeout.unwrap_or(DEFAULT_AUTH_TIMEOUT_SECS))
    }
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn parses_reauth_without_short_alias() {
        let result = Cli::try_parse_from(["q", "--reauth", "query"]);

        assert!(result.is_ok());
        if let Ok(cli) = result {
            assert!(cli.reauth);
        }
    }

    #[test]
    fn does_not_parse_login_alias() {
        assert!(Cli::try_parse_from(["q", "--login", "query"]).is_err());
    }

    #[test]
    fn parses_action_flags_without_query() {
        assert!(Cli::try_parse_from(["q", "--reauth"]).is_ok());
        assert!(Cli::try_parse_from(["q", "-l"]).is_ok());
        assert!(Cli::try_parse_from(["q", "--list-models"]).is_ok());
        assert!(Cli::try_parse_from(["q", "-u"]).is_ok());
        assert!(Cli::try_parse_from(["q", "--usage-limits"]).is_ok());
        assert!(Cli::try_parse_from(["q", "--rebuild-venv"]).is_ok());
        assert!(Cli::try_parse_from(["q", "--import-cookies"]).is_ok());
    }

    #[test]
    fn still_requires_query_for_normal_run() {
        assert!(Cli::try_parse_from(["q"]).is_err());
    }

    #[test]
    fn parses_auth_timeout() {
        let result = Cli::try_parse_from(["q", "--auth-timeout", "600", "query"]);
        assert!(result.is_ok());
        if let Ok(cli) = result {
            assert_eq!(cli.auth_timeout, Some(600));
            assert_eq!(cli.effective_auth_timeout().as_secs(), 600);
        }
    }

    #[test]
    fn rejects_invalid_auth_timeout() {
        assert!(Cli::try_parse_from(["q", "--auth-timeout", "abc", "query"]).is_err());
        assert!(Cli::try_parse_from(["q", "--auth-timeout", "0", "query"]).is_err());
        assert!(Cli::try_parse_from(["q", "--auth-timeout", "999999", "query"]).is_err());
    }

    #[test]
    fn parses_the_provider() {
        let result = Cli::try_parse_from(["q", "--provider", "openai", "query"]);

        assert!(result.is_ok());
        if let Ok(cli) = result {
            assert_eq!(cli.provider.as_deref(), Some("openai"));
            assert_eq!(cli.effective_provider(), "openai");
        }
    }

    #[test]
    fn the_provider_falls_back_to_the_cookie_one() {
        let result = Cli::try_parse_from(["q", "query"]);

        assert!(result.is_ok());
        if let Ok(cli) = result {
            assert_eq!(cli.effective_provider(), crate::provider::GOOGLE_COOKIES);
        }
    }

    #[test]
    fn the_api_management_needs_no_query() {
        let result = Cli::try_parse_from(["q", "--api-management"]);

        assert!(result.is_ok());
        if let Ok(cli) = result {
            assert!(cli.api_management);
        }
    }

    #[test]
    fn auth_timeout_falls_back_to_default() {
        let result = Cli::try_parse_from(["q", "query"]);
        assert!(result.is_ok());
        if let Ok(cli) = result {
            assert_eq!(cli.auth_timeout, None);
            assert_eq!(
                cli.effective_auth_timeout().as_secs(),
                crate::config::settings::DEFAULT_AUTH_TIMEOUT_SECS
            );
        }
    }
}

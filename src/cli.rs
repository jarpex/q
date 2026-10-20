use clap::Parser;

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

    /// The query text to send to the model.
    #[arg(
        required_unless_present_any = [
            "reauth",
            "list_models",
            "usage_limits",
            "rebuild_venv",
            "import_cookies",
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
}

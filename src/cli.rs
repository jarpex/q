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

    /// Open the browser to authenticate with Gemini.
    #[arg(short, long)]
    pub login: bool,

    /// The Gemini model to use for the query.
    #[arg(short, long, default_value = "gemini-flash")]
    pub model: String,

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
    #[arg(required_unless_present = "help")]
    pub query: Vec<String>,
}

use clap::Parser;

#[derive(Parser)]
#[command(
    name = env!("CARGO_PKG_NAME"),
    version = env!("CARGO_PKG_VERSION"),
    about = env!("CARGO_PKG_DESCRIPTION"),
    author = env!("CARGO_PKG_AUTHORS")
)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct Cli {
    /// Command mode: print only the requested command and copy to clipboard
    #[arg(short, long)]
    pub command_mode: bool,

    /// Force re-authentication with Gemini
    #[arg(short, long)]
    pub login: bool,

    /// Model to use (gemini-flash, gemini-pro, gemini-flash-lite)
    #[arg(short, long, default_value = "gemini-flash")]
    pub model: String,

    /// Disable streaming
    #[arg(long)]
    pub no_stream: bool,

    /// Enable debug output
    #[arg(short, long)]
    pub debug: bool,

    /// Force recreate Python virtual environment
    #[arg(long)]
    pub rebuild_venv: bool,

    /// Your query
    #[arg(required_unless_present = "help")]
    pub query: Vec<String>,
}
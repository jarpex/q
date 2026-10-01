use clap::Parser;

#[derive(Parser)]
#[command(
    name = env!("CARGO_PKG_NAME"),
    version = env!("CARGO_PKG_VERSION"),
    about = env!("CARGO_PKG_DESCRIPTION"),
    author = env!("CARGO_PKG_AUTHORS")
)]
#[allow(clippy::struct_excessive_bools)]
pub struct Cli {
    #[arg(short, long)]
    pub command_mode: bool,

    #[arg(short, long)]
    pub login: bool,

    #[arg(short, long, default_value = "gemini-flash")]
    pub model: String,

    #[arg(long)]
    pub no_stream: bool,

    #[arg(short, long)]
    pub debug: bool,

    #[arg(long)]
    pub rebuild_venv: bool,

    #[arg(required_unless_present = "help")]
    pub query: Vec<String>,
}

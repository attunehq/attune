use clap::{Parser, Subcommand};

mod cmd;

/// Attune CLI
///
/// Attune is the easiest way to securely publish Linux packages.
#[derive(Parser)]
#[command(name = "attune", max_term_width = 80)]
struct Args {
    /// Attune API token.
    #[arg(long, env = "ATTUNE_API_TOKEN")]
    api_token: Option<String>,

    /// Tool to run.
    #[command(subcommand)]
    tool: ToolCommand,
}

#[derive(Subcommand)]
enum ToolCommand {
    #[command(name = "apt", about = "Manage APT repositories")]
    Apt(cmd::apt::AptCommand),
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    match args.tool {
        ToolCommand::Apt(command) => cmd::apt::handle_apt(command).await,
    }
}

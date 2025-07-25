use clap::{Parser, Subcommand};

mod cmd;
mod config;

/// Attune CLI
///
/// Attune is the easiest way to securely publish Linux packages.
#[derive(Parser)]
#[command(name = "attune", max_term_width = 80)]
struct Args {
    /// Attune API token.
    #[arg(long, env = "ATTUNE_API_TOKEN")]
    api_token: String,

    /// Attune API endpoint.
    #[arg(long, env = "ATTUNE_API_ENDPOINT", default_value = "https://api.attunehq.com")]
    api_endpoint: String,

    /// Tool to run.
    #[command(subcommand)]
    tool: ToolCommand,
}

#[derive(Subcommand)]
enum ToolCommand {
    /// Manage APT repositories
    Apt(cmd::apt::AptCommand),
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let ctx = config::Config::new(args.api_token, args.api_endpoint);

    match args.tool {
        ToolCommand::Apt(command) => cmd::apt::handle_apt(ctx, command).await,
    }
}

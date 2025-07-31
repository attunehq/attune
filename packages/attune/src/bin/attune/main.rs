use std::process::ExitCode;

use attune::{api::ErrorResponse, server::compatibility::CompatibilityResponse};
use axum::http::StatusCode;
use clap::{Parser, Subcommand};
use colored::Colorize;
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

mod cmd;
mod config;

/// Attune CLI
///
/// Attune is the easiest way to securely publish Linux packages.
#[derive(Parser)]
#[command(version = "v0.1.0", name = "attune", max_term_width = 80)]
struct Args {
    /// Attune API token.
    #[arg(long, env = "ATTUNE_API_TOKEN")]
    api_token: String,

    /// Attune API endpoint.
    #[arg(
        long,
        env = "ATTUNE_API_ENDPOINT",
        default_value = "https://api.attunehq.com"
    )]
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
async fn main() -> ExitCode {
    // Set up logging.
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
                .with_file(true)
                .with_line_number(true)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_writer(std::io::stderr)
                .pretty(),
        )
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    let ctx = config::Config::new(args.api_token, args.api_endpoint);

    // Do a check for API version compatibility.
    let res = ctx
        .client
        .get(ctx.endpoint.join("/api/v0/compatibility").unwrap())
        .send()
        .await
        .expect("Could not reach API server");
    match res.status() {
        StatusCode::OK => {
            let compatibility = res
                .json::<CompatibilityResponse>()
                .await
                .expect("Could not parse compatibility response");
            match compatibility {
                CompatibilityResponse::Ok => {}
                CompatibilityResponse::WarnUpgrade { latest } => {
                    eprintln!("{} {}\n", "New version of attune available".blue(), latest);
                }
                CompatibilityResponse::Incompatible { minimum } => {
                    eprintln!(
                        "Error: CLI version is incompatible with API server. Please upgrade to version {minimum:?} or newer."
                    );
                    return ExitCode::FAILURE;
                }
            }
        }
        _ => {
            let err = res
                .json::<ErrorResponse>()
                .await
                .expect("Could not parse error response");
            eprintln!(
                "Error: could not check CLI version compatibility: {}",
                err.message
            );
            return ExitCode::FAILURE;
        }
    }

    // Execute subcommand.
    match args.tool {
        ToolCommand::Apt(command) => cmd::apt::handle_apt(ctx, command).await,
    }
}

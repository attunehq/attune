use std::{process::ExitCode, time::Duration};

use attune::{api::ErrorResponse, server::compatibility::CompatibilityResponse};
use axum::http::StatusCode;
use clap::{Parser, Subcommand};
use colored::Colorize;
use tracing::debug;
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

mod cmd;
mod config;

/// Attune CLI
///
/// Attune is the easiest way to securely publish Linux packages.
#[derive(Parser, Debug)]
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

#[derive(Subcommand, Debug)]
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
    debug!(?args, "parsed arguments");

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
    //
    // TODO: We should update all the subcommands to return `Result<String, ErrorResponse>`
    //       so that we can centralize retries, pretty printing, etc.
    match args.tool {
        ToolCommand::Apt(command) => cmd::apt::handle_apt(ctx, command).await,
    }
}

/// Infinitely retry an asynchronous function call.
///
/// - `operation` is the function to call.
/// - `should_retry` evaluates whether the operation should be retried.
/// - `retry_delay` provides the duration to wait before retrying.
///
/// Optionally, you can use [`retry_delay_default`] for default delay timings.
pub async fn retry_infinite<T, E>(
    operation: impl AsyncFn() -> Result<T, E>,
    should_retry: impl Fn(&E) -> bool,
    retry_delay: impl Fn(usize) -> Duration,
) -> Result<T, E> {
    for attempt in 0usize.. {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                if should_retry(&e) {
                    tokio::time::sleep(retry_delay(attempt)).await;
                } else {
                    return Err(e);
                }
            }
        }
    }
    unreachable!("loop is functionally infinite");
}

/// The default retry delay is a static delay of 2 seconds
/// plus a random jitter of up to 2 seconds.
pub fn retry_delay_default(_: usize) -> Duration {
    const STATIC_RETRY_DELAY_MS: u64 = 2000;
    Duration::from_millis(STATIC_RETRY_DELAY_MS + rand::random_range(0..STATIC_RETRY_DELAY_MS))
}

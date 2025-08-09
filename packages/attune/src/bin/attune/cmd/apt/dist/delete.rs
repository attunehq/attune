use std::process::ExitCode;

use clap::Args;
use colored::Colorize;
use inquire::Confirm;

use crate::{cmd::apt::dist::build_distribution_url, config::Config};
use attune::{api::ErrorResponse, server::repo::dist::delete::DeleteDistributionResponse};

#[derive(Args, Debug)]
pub struct DeleteArgs {
    /// The repository containing the distribution.
    #[arg(long)]
    repo: String,
    /// The name of the distribution to delete.
    #[arg(long)]
    name: String,
}

pub async fn run(ctx: Config, args: DeleteArgs) -> ExitCode {
    println!("{}", format!(
        "Warning: This will irreversibly delete distribution {:?} from repository {:?} and all its components, package indexes, and package associations.",
        args.name,
        args.repo
    ).red());

    let confirmed = Confirm::new("Are you sure you want to proceed?")
        .with_default(false)
        .prompt()
        .map_err(|e| {
            eprintln!("Confirmation failed: {e}");
            e
        });

    match confirmed {
        Ok(false) => {
            println!("Operation cancelled");
            return ExitCode::SUCCESS;
        }
        Err(_) => return ExitCode::FAILURE,
        Ok(true) => {}
    }

    loop {
        match delete_distribution(&ctx, &args).await {
            Ok(message) => {
                println!("{message}");
                return ExitCode::SUCCESS;
            }
            Err(error) => {
                if crate::retry::should_retry(&error) {
                    let delay = crate::retry::calculate_retry_delay();
                    tracing::warn!(?delay, ?error, "retrying: concurrent change");
                    tokio::time::sleep(delay).await;
                    continue;
                } else {
                    eprintln!("Error deleting distribution: {}", error.message);
                    return ExitCode::FAILURE;
                }
            }
        }
    }
}

async fn delete_distribution(ctx: &Config, args: &DeleteArgs) -> Result<String, ErrorResponse> {
    let url = build_distribution_url(ctx, &args.repo, Some(&args.name));
    let res = ctx.client.delete(url).send().await.map_err(|err| {
        ErrorResponse::builder()
            .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
            .error("REQUEST_FAILED")
            .message(format!("Failed to send request: {err}"))
            .build()
    })?;

    let status = res.status();
    if status == axum::http::StatusCode::OK {
        res.json::<DeleteDistributionResponse>()
            .await
            .map_err(|err| {
                ErrorResponse::builder()
                    .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                    .error("PARSE_ERROR")
                    .message(format!("Failed to parse response: {err}"))
                    .build()
            })?;
        Ok(format!("Distribution {:?} deleted successfully", args.name))
    } else {
        let error = res.json::<ErrorResponse>().await.map_err(|err| {
            ErrorResponse::builder()
                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                .error("PARSE_ERROR")
                .message(format!("Failed to parse error response: {err}"))
                .build()
        })?;
        Err(error)
    }
}

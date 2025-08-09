use std::process::ExitCode;

use clap::Args;

use crate::{cmd::apt::dist::build_distribution_url, config::Config};
use attune::{
    api::ErrorResponse,
    server::repo::dist::edit::{EditDistributionRequest, EditDistributionResponse},
};

#[derive(Args, Debug)]
pub struct EditArgs {
    /// The repository containing the distribution.
    #[arg(long)]
    repo: String,
    /// The name of the distribution to edit.
    #[arg(long)]
    name: String,
    /// Optional metadata to update.
    #[command(flatten)]
    metadata: EditMetadata,
}

#[derive(Args, Debug)]
pub struct EditMetadata {
    /// Update the distribution's description.
    #[arg(long)]
    description: Option<String>,
    /// Update the distribution's origin.
    #[arg(long)]
    origin: Option<String>,
    /// Update the distribution's label.
    #[arg(long)]
    label: Option<String>,
    /// Update the distribution's version.
    #[arg(long)]
    version: Option<String>,
    /// Update the distribution's suite.
    #[arg(long)]
    suite: Option<String>,
    /// Update the distribution's codename.
    #[arg(long)]
    codename: Option<String>,
}

pub async fn run(ctx: Config, args: EditArgs) -> ExitCode {
    let request = EditDistributionRequest::builder()
        .maybe_description(args.metadata.description.clone())
        .maybe_origin(args.metadata.origin.clone())
        .maybe_label(args.metadata.label.clone())
        .maybe_version(args.metadata.version.clone())
        .maybe_suite(args.metadata.suite.clone())
        .maybe_codename(args.metadata.codename.clone())
        .build();

    if !request.any_some() {
        eprintln!("No fields to update provided. Use --help to see available options.");
        return ExitCode::FAILURE;
    }

    loop {
        match edit_distribution(&ctx, &args, &request).await {
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
                    eprintln!("Error editing distribution: {}", error.message);
                    return ExitCode::FAILURE;
                }
            }
        }
    }
}

async fn edit_distribution(
    ctx: &Config,
    args: &EditArgs,
    request: &EditDistributionRequest,
) -> Result<String, ErrorResponse> {
    let url = build_distribution_url(ctx, &args.repo, Some(&args.name));
    let res = ctx
        .client
        .put(url)
        .json(request)
        .send()
        .await
        .map_err(|err| {
            ErrorResponse::builder()
                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                .error("REQUEST_FAILED")
                .message(format!("Failed to send request: {err}"))
                .build()
        })?;

    let status = res.status();
    if status == axum::http::StatusCode::OK {
        let response = res
            .json::<EditDistributionResponse>()
            .await
            .map_err(|err| {
                ErrorResponse::builder()
                    .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                    .error("PARSE_ERROR")
                    .message(format!("Failed to parse response: {err}"))
                    .build()
            })?;
        Ok(format!(
            concat!(
                "Distribution {:?} updated successfully\n",
                "Note: Changes will be reflected in repository indexes after the next sync."
            ),
            response.distribution
        ))
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

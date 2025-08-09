use std::process::ExitCode;

use clap::Args;

use crate::{cmd::apt::dist::build_distribution_url, config::Config};
use attune::{
    api::ErrorResponse,
    server::repo::dist::create::{CreateDistributionRequest, CreateDistributionResponse},
};

#[derive(Args, Debug)]
pub struct CreateArgs {
    /// The repository in which to create the distribution.
    #[arg(long)]
    repo: String,

    /// The name of the distribution.
    ///
    /// Releases have a primary name, called the "distribution".
    /// This defines their URL path under `/dists` within the repository.
    /// Generally, this should either be the suite (e.g., "stable") or codename (e.g., "bullseye").
    #[arg(long)]
    name: String,

    /// The suite name (e.g., "stable", "testing", "unstable").
    /// Defaults to the same value as `name` if not provided.
    #[arg(long)]
    suite: Option<String>,

    /// The codename (e.g., "bullseye", "bookworm", "jammy").
    /// Defaults to the same value as `name` if not provided.
    #[arg(long)]
    codename: Option<String>,

    /// Optional metadata for the distribution.
    #[command(flatten)]
    metadata: DistMetadata,
}

#[derive(Args, Debug)]
pub struct DistMetadata {
    /// The distribution's description.
    #[arg(long)]
    description: Option<String>,

    /// The distribution's origin (e.g., "Debian", "Ubuntu", your organization name).
    #[arg(long)]
    origin: Option<String>,

    /// The distribution's label (e.g., "Debian", "Ubuntu").
    #[arg(long)]
    label: Option<String>,

    /// The distribution's version (e.g., "11.0", "22.04").
    #[arg(long)]
    version: Option<String>,
}

pub async fn run(ctx: Config, args: CreateArgs) -> ExitCode {
    loop {
        match create_distribution(&ctx, &args).await {
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
                    eprintln!("Error creating distribution: {}", error.message);
                    return ExitCode::FAILURE;
                }
            }
        }
    }
}

async fn create_distribution(ctx: &Config, args: &CreateArgs) -> Result<String, ErrorResponse> {
    let request = CreateDistributionRequest::builder()
        .suite(args.suite.clone().unwrap_or_else(|| args.name.clone()))
        .codename(args.codename.clone().unwrap_or_else(|| args.name.clone()))
        .name(args.name.clone())
        .maybe_description(args.metadata.description.clone())
        .maybe_origin(args.metadata.origin.clone())
        .maybe_label(args.metadata.label.clone())
        .maybe_version(args.metadata.version.clone())
        .build();

    let url = build_distribution_url(ctx, &args.repo, None);
    let res = ctx
        .client
        .post(url)
        .json(&request)
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
            .json::<CreateDistributionResponse>()
            .await
            .map_err(|err| {
                ErrorResponse::builder()
                    .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                    .error("PARSE_ERROR")
                    .message(format!("Failed to parse response: {err}"))
                    .build()
            })?;
        Ok(format!(
            "Distribution {:?} created successfully",
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

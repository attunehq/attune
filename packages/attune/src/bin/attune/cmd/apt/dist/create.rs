use std::process::ExitCode;

use clap::Args;

use crate::config::Config;
use attune::server::repo::dist::create::{CreateDistributionRequest, CreateDistributionResponse};

use super::{build_distribution_url, handle_api_response};

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
    #[arg(long)]
    suite: String,

    /// The codename (e.g., "bullseye", "bookworm", "jammy").
    #[arg(long)]
    codename: String,

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
    let request = CreateDistributionRequest::builder()
        .name(args.name)
        .suite(args.suite)
        .codename(args.codename)
        .maybe_description(args.metadata.description)
        .maybe_origin(args.metadata.origin)
        .maybe_label(args.metadata.label)
        .maybe_version(args.metadata.version)
        .build();

    let url = build_distribution_url(&ctx, &args.repo, None);
    let response = ctx.client.post(url).json(&request).send().await;

    match response {
        Ok(resp) => match handle_api_response::<CreateDistributionResponse>(resp).await {
            Ok(CreateDistributionResponse { distribution, .. }) => {
                println!("Distribution '{distribution}' created successfully");
                ExitCode::SUCCESS
            }
            Err(exit_code) => exit_code,
        },
        Err(e) => {
            eprintln!("Failed to send request: {e}");
            ExitCode::FAILURE
        }
    }
}

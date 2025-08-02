use clap::Args;

use crate::{
    cmd::apt::dist::{build_distribution_url, handle_api_response},
    config::Config,
};
use attune::server::repo::dist::create::{CreateDistributionRequest, CreateDistributionResponse};

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

pub async fn run(ctx: Config, args: CreateArgs) -> Result<String, String> {
    let request = CreateDistributionRequest::builder()
        .suite(args.suite.unwrap_or_else(|| args.name.clone()))
        .codename(args.codename.unwrap_or_else(|| args.name.clone()))
        .name(args.name)
        .maybe_description(args.metadata.description)
        .maybe_origin(args.metadata.origin)
        .maybe_label(args.metadata.label)
        .maybe_version(args.metadata.version)
        .build();

    let url = build_distribution_url(&ctx, &args.repo, None);
    ctx.client
        .post(url)
        .json(&request)
        .send()
        .await
        .map(handle_api_response::<CreateDistributionResponse>)
        .map_err(|err| format!("Failed to send request: {err}"))?
        .await
        .map(|CreateDistributionResponse { distribution, .. }| {
            format!("Distribution {distribution:?} created successfully")
        })
}

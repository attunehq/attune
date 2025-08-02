use clap::Args;

use crate::{
    cmd::apt::dist::{build_distribution_url, handle_api_response},
    config::Config,
};
use attune::server::repo::dist::edit::{EditDistributionRequest, EditDistributionResponse};

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

pub async fn run(ctx: Config, args: EditArgs) -> Result<String, String> {
    let request = EditDistributionRequest::builder()
        .maybe_description(args.metadata.description)
        .maybe_origin(args.metadata.origin)
        .maybe_label(args.metadata.label)
        .maybe_version(args.metadata.version)
        .maybe_suite(args.metadata.suite)
        .maybe_codename(args.metadata.codename)
        .build();

    if !request.any_some() {
        return Err(String::from(
            "No fields to update provided. Use --help to see available options.",
        ));
    }

    let url = build_distribution_url(&ctx, &args.repo, Some(&args.name));
    ctx.client
        .put(url)
        .json(&request)
        .send()
        .await
        .map(handle_api_response::<EditDistributionResponse>)
        .map_err(|err| format!("Failed to send request: {err}"))?
        .await
        .map(|EditDistributionResponse { distribution, .. }| {
            format!(
                concat!(
                    "Distribution {:?} updated successfully\n",
                    "Note: Changes will be reflected in repository indexes after the next sync."
                ),
                distribution
            )
        })
}

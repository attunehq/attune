use std::process::ExitCode;

use clap::Args;

use crate::config::Config;
use attune::server::repo::dist::edit::{EditDistributionRequest, EditDistributionResponse};

use super::{build_distribution_url, handle_api_response};

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
    let request = EditDistributionRequest {
        description: args.metadata.description,
        origin: args.metadata.origin,
        label: args.metadata.label,
        version: args.metadata.version,
        suite: args.metadata.suite,
        codename: args.metadata.codename,
    };

    // Check if any fields were provided
    if request.description.is_none() && request.origin.is_none() && request.label.is_none() 
        && request.version.is_none() && request.suite.is_none() && request.codename.is_none() {
        eprintln!("No fields to update provided. Use --help to see available options.");
        return ExitCode::FAILURE;
    }

    let url = build_distribution_url(&ctx, &args.repo, Some(&args.name));
    let response = ctx
        .client
        .put(url)
        .json(&request)
        .send()
        .await;

    match response {
        Ok(resp) => match handle_api_response::<EditDistributionResponse>(resp).await {
            Ok(dist) => {
                println!("Distribution '{}' updated successfully", dist.distribution);
                println!("Note: Changes will be reflected in repository indexes after the next sync.");
                ExitCode::SUCCESS
            }
            Err(exit_code) => exit_code,
        },
        Err(e) => {
            eprintln!("Failed to send request: {}", e);
            ExitCode::FAILURE
        }
    }
}

use std::process::ExitCode;

use clap::Args;

use crate::config::Config;

#[derive(Args, Debug)]
pub struct CreateArgs {
    /// The repository in which to create the distribution.
    #[arg(long)]
    repo: String,
    /// The name of the distribution.
    ///
    /// Releases have a primary name, called the "distribution".
    /// This defines their URL path under `/dists` within the repository.
    #[arg(long)]
    name: String,
    /// The distribution's metadata.
    #[command(flatten)]
    metadata: DistMetadata,
}

#[derive(Args, Debug)]
pub struct DistMetadata {
    /// The distribution's description.
    #[arg(long)]
    description: Option<String>,
    /// The distribution's origin.
    #[arg(long)]
    origin: Option<String>,
    /// The distribution's label.
    #[arg(long)]
    label: Option<String>,
    /// The distribution's version.
    #[arg(long)]
    version: Option<String>,
}

pub async fn run(ctx: Config, args: CreateArgs) -> ExitCode {
    println!("Creating distribution: {args:?}");
    ExitCode::FAILURE
}

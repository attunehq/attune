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
    println!("Creating distribution: {args:?}");
    ExitCode::FAILURE
}

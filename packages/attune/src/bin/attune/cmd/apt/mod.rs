use std::process::ExitCode;

use clap::{Args, Subcommand};

use crate::config::Config;

mod dist;
mod pkg;
mod repo;

#[derive(Args, Debug)]
pub struct AptCommand {
    #[command(subcommand)]
    subcommand: AptSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum AptSubcommand {
    /// Create and edit repositories
    #[command(visible_alias = "repo")]
    Repository(repo::RepoCommand),
    /// Manage distributions
    ///
    /// You generally shouldn't need to use these commands unless you're doing
    /// something advanced. Distributions are automatically created as needed
    /// when packages are uploaded into them.
    #[command(visible_alias = "dist")]
    Distribution(dist::DistCommand),
    /// Publish packages
    #[command(visible_alias = "pkg")]
    Package(pkg::PkgCommand),
}

pub async fn handle_apt(ctx: Config, command: AptCommand) -> ExitCode {
    match command.subcommand {
        AptSubcommand::Repository(repo) => repo::handle_repo(ctx, repo).await,
        AptSubcommand::Distribution(dist) => dist::handle_dist(ctx, dist).await,
        AptSubcommand::Package(pkg) => pkg::handle_pkg(ctx, pkg).await,
    }
}

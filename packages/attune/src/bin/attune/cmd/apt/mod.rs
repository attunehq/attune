use std::process::ExitCode;

use clap::{Args, Subcommand};

use crate::config::Config;

mod dist;
mod pkg;
mod repo;

#[derive(Args)]
pub struct AptCommand {
    #[command(subcommand)]
    subcommand: AptSubcommand,
}

#[derive(Subcommand)]
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
        AptSubcommand::Package(pkg) => pkg::handle_pkg(ctx, pkg).await,
        // Here we handle the error responses to transform them into the way other subcommands work,
        // if we want to later we can do the same for other subcommands.
        //
        // Also, if we really want to make this nice, we can convert to `color-eyre`.
        AptSubcommand::Distribution(dist) => match dist::handle_dist(ctx, dist).await {
            Ok(output) => {
                println!("{output}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("Error: {err}");
                ExitCode::FAILURE
            }
        },
    }
}

use clap::{Args, Subcommand};

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

pub async fn handle_apt(command: AptCommand) {
    match command.subcommand {
        AptSubcommand::Repository(repo) => repo::handle_repo(repo).await,
        AptSubcommand::Distribution(dist) => dist::handle_dist(dist).await,
        AptSubcommand::Package(pkg) => pkg::handle_pkg(pkg).await,
    }
}

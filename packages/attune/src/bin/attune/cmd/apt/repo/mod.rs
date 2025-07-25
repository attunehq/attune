use clap::{Args, Subcommand};

use crate::config::Config;

mod create;

#[derive(Args)]
pub struct RepoCommand {
    #[command(subcommand)]
    subcommand: RepoSubCommand,
}

#[derive(Subcommand)]
pub enum RepoSubCommand {
    /// Create a new repository
    #[command(visible_aliases = ["new", "add"])]
    Create(create::RepoCreateCommand),
    /// Show information about repositories
    #[command(visible_alias = "ls")]
    List,
    /// Edit repository metadata
    #[command(visible_alias = "set")]
    Edit,
    /// Delete a repository
    #[command(visible_alias = "rm")]
    Delete,
}

pub async fn handle_repo(ctx: Config, command: RepoCommand) {
    match command.subcommand {
        RepoSubCommand::Create(create) => create::handle_repo_create(ctx, create).await,
        RepoSubCommand::List => println!("Listing repositories"),
        RepoSubCommand::Edit => println!("Editing repository"),
        RepoSubCommand::Delete => println!("Deleting repository"),
    }
}

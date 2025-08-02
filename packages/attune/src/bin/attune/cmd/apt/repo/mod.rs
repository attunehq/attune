use std::process::ExitCode;

use clap::{Args, Subcommand};

use crate::config::Config;

mod create;
mod delete;
mod edit;
mod list;

#[derive(Args, Debug)]
pub struct RepoCommand {
    #[command(subcommand)]
    subcommand: RepoSubCommand,
}

#[derive(Subcommand, Debug)]
pub enum RepoSubCommand {
    /// Create a new repository
    #[command(visible_aliases = ["new", "add"])]
    Create(create::RepoCreateCommand),
    /// Show information about repositories
    #[command(visible_alias = "ls")]
    List(list::RepoListCommand),
    /// Edit repository metadata
    #[command(visible_alias = "set")]
    Edit(edit::RepoEditCommand),
    /// Delete a repository
    #[command(visible_alias = "rm")]
    Delete(delete::RepoDeleteCommand),
    // TODO: Add a command that allows us to do a no-op re-signing of the index,
    // potentially with a different key.

    // TODO: Add a command that forcibly resynchronizes the repository state
    // with S3.
}

pub async fn handle_repo(ctx: Config, command: RepoCommand) -> ExitCode {
    match command.subcommand {
        RepoSubCommand::Create(create) => create::run(ctx, create).await,
        RepoSubCommand::List(list) => list::run(ctx, list).await,
        RepoSubCommand::Edit(edit) => edit::run(ctx, edit).await,
        RepoSubCommand::Delete(delete) => delete::run(ctx, delete).await,
    }
}

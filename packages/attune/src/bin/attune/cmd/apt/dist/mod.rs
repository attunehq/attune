use std::process::ExitCode;

use clap::{Args, Subcommand};

use crate::config::Config;

mod create;
mod delete;
mod edit;
mod list;

#[derive(Args)]
pub struct DistCommand {
    #[command(subcommand)]
    subcommand: DistSubCommand,
}

#[derive(Subcommand)]
pub enum DistSubCommand {
    /// Create a new distribution
    ///
    /// For details on the meanings of distribution ("Release") metadata fields,
    /// see <https://wiki.debian.org/DebianRepository/Format>.
    #[command(visible_aliases = ["new", "add"])]
    Create(create::CreateArgs),
    /// Show information about distributions
    #[command(visible_alias = "ls")]
    List(list::ListArgs),
    /// Edit distribution metadata
    ///
    /// For details on the meanings of distribution ("Release") metadata fields,
    /// see <https://wiki.debian.org/DebianRepository/Format>.
    ///
    /// Note that this will not actually update the published Release file until
    /// the next time you publish a package.
    #[command(visible_alias = "set")]
    Edit(edit::EditArgs),
    /// Delete a distribution
    #[command(visible_alias = "rm")]
    Delete(delete::DeleteArgs),
}

pub async fn handle_dist(ctx: Config, command: DistCommand) -> ExitCode {
    match command.subcommand {
        DistSubCommand::Create(args) => create::run(ctx, args).await,
        DistSubCommand::List(args) => list::run(ctx, args).await,
        DistSubCommand::Edit(args) => edit::run(ctx, args).await,
        DistSubCommand::Delete(args) => delete::run(ctx, args).await,
    }
}

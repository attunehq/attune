use std::process::ExitCode;

use attune::cli::apt::pkg::add::CmdAptPkgAdd;
use clap::{Args, Subcommand};

use crate::config::Config;

mod add;
mod list;
mod remove;

#[derive(Args, Debug)]
pub struct PkgCommand {
    #[command(subcommand)]
    subcommand: PkgSubCommand,
}

#[derive(Subcommand, Debug)]
pub enum PkgSubCommand {
    /// Upload a new package
    #[command(visible_aliases = ["new", "upload"])]
    Add(CmdAptPkgAdd),
    /// Show information about packages
    #[command(visible_alias = "ls")]
    List(list::PkgListCommand),
    /// Remove a package
    #[command(visible_aliases = ["rm", "delete"])]
    Remove(remove::PkgRemoveCommand),
}

pub async fn handle_pkg(ctx: Config, command: PkgCommand) -> ExitCode {
    match command.subcommand {
        PkgSubCommand::Add(add) => add::run(ctx, add).await,
        PkgSubCommand::List(list) => list::run(ctx, list).await,
        PkgSubCommand::Remove(remove) => remove::run(ctx, remove).await,
    }
}

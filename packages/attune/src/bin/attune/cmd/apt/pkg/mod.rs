use std::process::ExitCode;

use clap::{Args, Subcommand};

use crate::config::Config;

mod add;
mod list;

#[derive(Args)]
pub struct PkgCommand {
    #[command(subcommand)]
    subcommand: PkgSubCommand,
}

#[derive(Subcommand)]
pub enum PkgSubCommand {
    /// Upload a new package
    #[command(visible_aliases = ["new", "upload"])]
    Add(add::PkgAddCommand),
    /// Show information about packages
    #[command(visible_alias = "ls")]
    List(list::PkgListCommand),
    /// Remove a package
    #[command(visible_aliases = ["rm", "delete"])]
    Remove,
}

pub async fn handle_pkg(ctx: Config, command: PkgCommand) -> ExitCode {
    match command.subcommand {
        PkgSubCommand::Add(add) => add::run(ctx, add).await,
        PkgSubCommand::List(list) => list::run(ctx, list).await,
        PkgSubCommand::Remove => {
            println!("Removing package");
            ExitCode::FAILURE
        }
    }
}

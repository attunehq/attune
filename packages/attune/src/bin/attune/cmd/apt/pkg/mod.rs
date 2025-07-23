use clap::{Args, Subcommand};

#[derive(Args)]
pub struct PkgCommand {
    #[command(subcommand)]
    subcommand: PkgSubCommand,
}

#[derive(Subcommand)]
pub enum PkgSubCommand {
    /// Upload a new package
    #[command(visible_aliases = ["new", "upload"])]
    Add,
    /// Show information about packages
    #[command(visible_alias = "ls")]
    List,
    /// Remove a package
    #[command(visible_aliases = ["rm", "delete"])]
    Remove,
}

pub async fn handle_pkg(command: PkgCommand) {
    match command.subcommand {
        PkgSubCommand::Add => {}
        PkgSubCommand::List => println!("Listing packages"),
        PkgSubCommand::Remove => println!("Removing package"),
    }
}

use clap::{Args, Subcommand};

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
    Create,
    /// Show information about distributions
    #[command(visible_alias = "ls")]
    List,
    /// Edit distribution metadata
    ///
    /// For details on the meanings of distribution ("Release") metadata fields,
    /// see <https://wiki.debian.org/DebianRepository/Format>.
    #[command(visible_alias = "set")]
    Edit,
    /// Delete a distribution
    #[command(visible_alias = "rm")]
    Delete,
}

pub async fn handle_dist(command: DistCommand) {
    match command.subcommand {
        DistSubCommand::Create => {}
        DistSubCommand::List => println!("Listing distributions"),
        DistSubCommand::Edit => println!("Editing distribution"),
        DistSubCommand::Delete => println!("Deleting distribution"),
    }
}

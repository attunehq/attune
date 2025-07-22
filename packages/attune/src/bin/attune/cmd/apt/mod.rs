use clap::{Args, Subcommand};

#[derive(Args)]
pub struct AptCommand {
    #[command(subcommand)]
    subcommand: AptSubcommand,
}

#[derive(Subcommand)]
pub enum AptSubcommand {
    #[command(name = "repository", visible_alias = "repo", about = "Create and edit repositories")]
    Repository,
    #[command(name = "distribution", visible_alias = "dist", about = "Manage distributions")]
    Distribution,
    #[command(name = "package", visible_alias = "pkg", about = "Publish packages")]
    Package,
}

pub async fn handle_apt(command: AptCommand) {
    match command.subcommand {
        AptSubcommand::Repository => println!("Managing repositories"),
        AptSubcommand::Distribution => println!("Managing distributions"),
        AptSubcommand::Package => println!("Managing packages"),
    }
}

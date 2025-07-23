use clap::Args;

#[derive(Args)]
pub struct RepoCreateCommand {
    /// A name that uniquely identifies this repository.
    name: String,
}

pub async fn handle_repo_create(command: RepoCreateCommand) {
    todo!();
}

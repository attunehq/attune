use std::process::ExitCode;

use clap::Args;

use crate::config::Config;

#[derive(Args, Debug)]
pub struct ListArgs {
    /// The name of the repository.
    #[arg(long)]
    repo: String,
}

pub async fn run(ctx: Config, args: ListArgs) -> ExitCode {
    println!("Listing distributions for repository: {args:?}");
    ExitCode::FAILURE
}

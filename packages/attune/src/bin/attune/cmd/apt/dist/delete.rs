use std::process::ExitCode;

use clap::Args;

use crate::config::Config;

#[derive(Args, Debug)]
pub struct DeleteArgs {
    /// The name of the distribution.
    #[arg(long)]
    name: String,
}

pub async fn run(ctx: Config, args: DeleteArgs) -> ExitCode {
    println!("Deleting distribution: {args:?}");
    ExitCode::FAILURE
}

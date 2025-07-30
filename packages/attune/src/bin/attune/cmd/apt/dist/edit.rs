use std::process::ExitCode;

use clap::Args;

use crate::config::Config;

#[derive(Args, Debug)]
pub struct EditArgs {
    /// The name of the distribution.
    #[arg(long)]
    name: String,
}

pub async fn run(ctx: Config, args: EditArgs) -> ExitCode {
    println!("Editing distribution: {args:?}");
    ExitCode::FAILURE
}

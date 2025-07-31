use clap::Args;
use colored::Colorize;
use inquire::Confirm;

use crate::{
    cmd::apt::dist::{build_distribution_url, handle_api_response},
    config::Config,
};
use attune::server::repo::dist::delete::DeleteDistributionResponse;

macro_rules! println_warning {
    ($($arg:tt)*) => {
        println!("Warning: {}", format!($($arg)*).red());
    };
}

#[derive(Args, Debug)]
pub struct DeleteArgs {
    /// The repository containing the distribution.
    #[arg(long)]
    repo: String,
    /// The name of the distribution to delete.
    #[arg(long)]
    name: String,
}

pub async fn run(ctx: Config, args: DeleteArgs) -> Result<String, String> {
    println_warning!(
        "This will irreversibly delete distribution {:?} from repository {:?} and all its components, package indexes, and package associations.",
        args.name,
        args.repo
    );

    let confirmed = Confirm::new("Are you sure you want to proceed?")
        .with_default(false)
        .prompt()
        .map_err(|e| format!("Confirmation failed: {e}"))?;
    if !confirmed {
        return Ok(String::from("Operation cancelled"));
    }

    let url = build_distribution_url(&ctx, &args.repo, Some(&args.name));
    ctx.client
        .delete(url)
        .send()
        .await
        .map(handle_api_response::<DeleteDistributionResponse>)
        .map_err(|err| format!("Failed to send request: {err}"))?
        .await
        .map(|_| format!("Distribution {:?} deleted successfully", args.name))
}

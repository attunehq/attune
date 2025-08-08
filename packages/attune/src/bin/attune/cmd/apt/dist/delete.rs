use clap::Args;
use colored::Colorize;
use inquire::Confirm;

use crate::{cmd::apt::dist::build_distribution_url, config::Config};
use attune::server::repo::dist::delete::DeleteDistributionResponse;

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
    println!("{}", format!(
        "Warning: This will irreversibly delete distribution {:?} from repository {:?} and all its components, package indexes, and package associations.",
        args.name,
        args.repo
    ).red());

    let confirmed = Confirm::new("Are you sure you want to proceed?")
        .with_default(false)
        .prompt()
        .map_err(|e| format!("Confirmation failed: {e}"))?;
    if !confirmed {
        return Ok(String::from("Operation cancelled"));
    }

    let url = build_distribution_url(&ctx, &args.repo, Some(&args.name));
    crate::http::delete::<DeleteDistributionResponse>(&ctx, &url)
        .await
        .map_err(|err| format!("API error: {}", err.message))
        .map(|_| format!("Distribution {:?} deleted successfully", args.name))
}

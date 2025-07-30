use std::process::ExitCode;

use clap::Args;

use crate::config::Config;
use attune::server::repo::dist::delete::DeleteDistributionResponse;

use super::{build_distribution_url, handle_api_response, confirm_destructive_action};

#[derive(Args, Debug)]
pub struct DeleteArgs {
    /// The repository containing the distribution.
    #[arg(long)]
    repo: String,
    /// The name of the distribution to delete.
    #[arg(long)]
    name: String,
}

pub async fn run(ctx: Config, args: DeleteArgs) -> ExitCode {
    let warning_message = format!(
        "this will irreversibly delete distribution '{}' from repository '{}' and all its components, package indexes, and package associations",
        args.name, args.repo
    );

    match confirm_destructive_action(&warning_message) {
        Ok(true) => {}, // User confirmed, proceed
        Ok(false) => {
            println!("Operation cancelled");
            return ExitCode::SUCCESS;
        },
        Err(exit_code) => return exit_code,
    }

    let url = build_distribution_url(&ctx, &args.repo, Some(&args.name));
    let response = ctx
        .client
        .delete(url)
        .send()
        .await;

    match response {
        Ok(resp) => match handle_api_response::<DeleteDistributionResponse>(resp).await {
            Ok(_) => {
                println!("Distribution '{}' deleted successfully", args.name);
                ExitCode::SUCCESS
            }
            Err(exit_code) => exit_code,
        },
        Err(e) => {
            eprintln!("Failed to send request: {}", e);
            ExitCode::FAILURE
        }
    }
}

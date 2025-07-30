use std::process::ExitCode;

use clap::Args;
use tabled::settings::Style;

use crate::config::Config;
use attune::server::repo::dist::list::ListDistributionsResponse;

use super::{build_distribution_url, handle_api_response};

#[derive(Args, Debug)]
pub struct ListArgs {
    /// The name of the repository.
    #[arg(long)]
    repo: String,
}

pub async fn run(ctx: Config, args: ListArgs) -> ExitCode {
    let url = build_distribution_url(&ctx, &args.repo, None);
    let response = ctx
        .client
        .get(url)
        .send()
        .await;

    match response {
        Ok(resp) => match handle_api_response::<ListDistributionsResponse>(resp).await {
            Ok(list_response) => {
                if list_response.distributions.is_empty() {
                    println!("No distributions found in repository '{}'", args.repo);
                    return ExitCode::SUCCESS;
                }

                let mut builder = tabled::builder::Builder::new();
                builder.push_record(["Name", "Suite", "Codename", "Description"]);
                
                for dist in list_response.distributions {
                    builder.push_record([
                        &dist.distribution,
                        &dist.suite,
                        &dist.codename,
                        &dist.description.unwrap_or_else(|| "-".to_string()),
                    ]);
                }
                
                let mut table = builder.build();
                table.with(Style::modern());
                println!("{}", table);
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

use std::process::ExitCode;

use axum::http::StatusCode;
use clap::Args;
use colored::Colorize as _;
use inquire::Confirm;
use percent_encoding::percent_encode;

use crate::config::Config;
use attune::{
    api::{ErrorResponse, PATH_SEGMENT_PERCENT_ENCODE_SET},
    server::repo::delete::{DeleteRepositoryRequest, DeleteRepositoryResponse},
};

#[derive(Args, Debug)]
pub struct RepoDeleteCommand {
    /// The name of the repository to delete.
    name: String,

    /// Skip confirmation prompt and proceed with deletion
    #[arg(short, long)]
    yes: bool,
}

pub async fn run(ctx: Config, command: RepoDeleteCommand) -> ExitCode {
    println!(
        "{}",
        format!(
            "Warning: this will irreversibly delete repository {:?}",
            command.name
        )
        .on_red()
    );

    if !command.yes {
        let confirm = Confirm::new("Are you sure you want to proceed?")
            .with_default(false)
            .prompt();
        match confirm {
            Ok(true) => {}
            Ok(false) => return ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Aborting: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    let res = ctx
        .client
        .delete(
            ctx.endpoint
                .join(
                    format!(
                        "/api/v0/repositories/{}",
                        percent_encode(command.name.as_bytes(), PATH_SEGMENT_PERCENT_ENCODE_SET)
                    )
                    .as_str(),
                )
                .unwrap(),
        )
        .json(&DeleteRepositoryRequest {})
        .send()
        .await
        .expect("Could not send API request");
    match res.status() {
        StatusCode::OK => {
            res.json::<DeleteRepositoryResponse>()
                .await
                .expect("Could not parse response");
            println!("Repository deleted");
            ExitCode::SUCCESS
        }
        _ => {
            let error = res
                .json::<ErrorResponse>()
                .await
                .expect("Could not parse error response");
            eprintln!("Error deleting repository: {}", error.message);
            ExitCode::FAILURE
        }
    }
}

use std::process::ExitCode;

use axum::http::StatusCode;
use clap::Args;
use percent_encoding::percent_encode;

use crate::config::Config;
use attune::{
    api::{ErrorResponse, PATH_SEGMENT_PERCENT_ENCODE_SET},
    server::repo::edit::{EditRepositoryRequest, EditRepositoryResponse},
};

#[derive(Args, Debug)]
pub struct RepoEditCommand {
    /// The name of the repository to edit.
    #[arg(long)]
    name: String,

    /// The new name for the repository.
    #[arg(long)]
    new_name: Option<String>,
}

pub async fn run(ctx: Config, command: RepoEditCommand) -> ExitCode {
    let res = ctx
        .client
        .put(
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
        .json(&EditRepositoryRequest {
            new_name: command.new_name,
        })
        .send()
        .await
        .expect("Could not send API request");
    match res.status() {
        StatusCode::OK => {
            let repo = res
                .json::<EditRepositoryResponse>()
                .await
                .expect("Could not parse response");
            println!("Repository edited: {:?}", repo.result);
            ExitCode::SUCCESS
        }
        _ => {
            let error = res
                .json::<ErrorResponse>()
                .await
                .expect("Could not parse error response");
            eprintln!("Error editing repository: {}", error.message);
            ExitCode::FAILURE
        }
    }
}

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
    loop {
        match edit_repository(&ctx, &command).await {
            Ok(message) => {
                println!("{message}");
                return ExitCode::SUCCESS;
            }
            Err(error) => {
                if crate::retry::should_retry(&error) {
                    let delay = crate::retry::calculate_retry_delay();
                    tracing::warn!(?delay, ?error, "retrying: concurrent change");
                    tokio::time::sleep(delay).await;
                    continue;
                } else {
                    eprintln!("Error editing repository: {}", error.message);
                    return ExitCode::FAILURE;
                }
            }
        }
    }
}

async fn edit_repository(ctx: &Config, command: &RepoEditCommand) -> Result<String, ErrorResponse> {
    let url = ctx
        .endpoint
        .join(
            format!(
                "/api/v0/repositories/{}",
                percent_encode(command.name.as_bytes(), PATH_SEGMENT_PERCENT_ENCODE_SET)
            )
            .as_str(),
        )
        .map_err(|err| {
            ErrorResponse::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .error("URL_ERROR")
                .message(format!("Failed to build URL: {err}"))
                .build()
        })?;

    let res = ctx
        .client
        .put(url)
        .json(&EditRepositoryRequest {
            new_name: command.new_name.clone(),
        })
        .send()
        .await
        .map_err(|err| {
            ErrorResponse::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .error("REQUEST_FAILED")
                .message(format!("Failed to send request: {err}"))
                .build()
        })?;

    let status = res.status();
    if status == StatusCode::OK {
        let repo = res.json::<EditRepositoryResponse>().await.map_err(|err| {
            ErrorResponse::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .error("PARSE_ERROR")
                .message(format!("Failed to parse response: {err}"))
                .build()
        })?;
        Ok(format!(
            "Repository name changed from {:?} to {:?}",
            command.name, repo.result.name
        ))
    } else {
        let error = res.json::<ErrorResponse>().await.map_err(|err| {
            ErrorResponse::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .error("PARSE_ERROR")
                .message(format!("Failed to parse error response: {err}"))
                .build()
        })?;
        Err(error)
    }
}

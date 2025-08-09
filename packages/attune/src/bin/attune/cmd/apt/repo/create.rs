use std::process::ExitCode;

use axum::http::StatusCode;
use clap::Args;

use crate::config::Config;
use attune::{
    api::ErrorResponse,
    server::repo::create::{CreateRepositoryRequest, CreateRepositoryResponse},
};

#[derive(Args, Debug)]
pub struct RepoCreateCommand {
    /// A name that uniquely identifies this repository.
    name: String,
}

pub async fn run(ctx: Config, command: RepoCreateCommand) -> ExitCode {
    loop {
        match create_repository(&ctx, &command).await {
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
                    eprintln!("Error creating repository: {}", error.message);
                    return ExitCode::FAILURE;
                }
            }
        }
    }
}

async fn create_repository(
    ctx: &Config,
    command: &RepoCreateCommand,
) -> Result<String, ErrorResponse> {
    let url = ctx.endpoint.join("/api/v0/repositories").map_err(|err| {
        ErrorResponse::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .error("URL_ERROR")
            .message(format!("Failed to build URL: {err}"))
            .build()
    })?;

    let res = ctx
        .client
        .post(url)
        .json(&CreateRepositoryRequest {
            name: command.name.clone(),
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
        let repo = res
            .json::<CreateRepositoryResponse>()
            .await
            .map_err(|err| {
                ErrorResponse::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .error("PARSE_ERROR")
                    .message(format!("Failed to parse response: {err}"))
                    .build()
            })?;
        Ok(format!("Repository created: {}", repo.name))
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

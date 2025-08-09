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

    loop {
        match delete_repository(&ctx, &command).await {
            Ok(message) => {
                println!("{message}");
                return ExitCode::SUCCESS;
            }
            Err(error) => if crate::retry::should_retry(&error) {
                let delay = crate::retry::calculate_retry_delay();
                tracing::warn!(?delay, ?error, "retrying: concurrent change");
                tokio::time::sleep(delay).await;
                continue;
            } else {
                eprintln!("Error deleting repository: {}", error.message);
                return ExitCode::FAILURE;
            },
        }
    }
}

async fn delete_repository(ctx: &Config, command: &RepoDeleteCommand) -> Result<String, ErrorResponse> {
    let url = ctx.endpoint
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
        .delete(url)
        .json(&DeleteRepositoryRequest {})
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
        res.json::<DeleteRepositoryResponse>()
            .await
            .map_err(|err| {
                ErrorResponse::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .error("PARSE_ERROR")
                    .message(format!("Failed to parse response: {err}"))
                    .build()
            })?;
        Ok(String::from("Repository deleted"))
    } else {
        let error = res
            .json::<ErrorResponse>()
            .await
            .map_err(|err| {
                ErrorResponse::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .error("PARSE_ERROR")
                    .message(format!("Failed to parse error response: {err}"))
                    .build()
            })?;
        Err(error)
    }
}

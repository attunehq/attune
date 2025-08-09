use std::process::ExitCode;

use axum::http::StatusCode;
use clap::Args;
use percent_encoding::percent_encode;

use crate::config::Config;
use attune::{
    api::{ErrorResponse, PATH_SEGMENT_PERCENT_ENCODE_SET},
    server::repo::sync::resync::ResyncRepositoryResponse,
};

#[derive(Args, Debug)]
pub struct DistResyncCommand {
    /// The repository containing the distribution.
    #[arg(long)]
    repo: String,
    /// The name of the distribution to resync.
    #[arg(long)]
    name: String,
}

// TODO: We should move this command behind an EE or self-hosted build of the
// CLI, because it doesn't make sense for cloud-hosted users to see this
// command.
pub async fn run(ctx: Config, cmd: DistResyncCommand) -> ExitCode {
    loop {
        match resync_distribution(&ctx, &cmd).await {
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
                eprintln!("Error resyncing distribution: {}", error.message);
                return ExitCode::FAILURE;
            },
        }
    }
}

async fn resync_distribution(ctx: &Config, cmd: &DistResyncCommand) -> Result<String, ErrorResponse> {
    let url = ctx.endpoint
        .join(&format!(
            "/api/v0/repositories/{}/distributions/{}/sync",
            percent_encode(cmd.repo.as_bytes(), PATH_SEGMENT_PERCENT_ENCODE_SET),
            percent_encode(cmd.name.as_bytes(), PATH_SEGMENT_PERCENT_ENCODE_SET)
        ))
        .map_err(|err| {
            ErrorResponse::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .error("URL_ERROR")
                .message(format!("Failed to build URL: {err}"))
                .build()
        })?;

    let res = ctx
        .client
        .get(url)
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
        let _repo = res
            .json::<ResyncRepositoryResponse>()
            .await
            .map_err(|err| {
                ErrorResponse::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .error("PARSE_ERROR")
                    .message(format!("Failed to parse response: {err}"))
                    .build()
            })?;
        // TODO: Print something informative about what was resynchronized.
        Ok(format!("Distribution {:?} resynced!", cmd.name))
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

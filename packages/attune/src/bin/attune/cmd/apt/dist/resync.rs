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
pub async fn run(ctx: Config, cmd: DistResyncCommand) -> Result<String, String> {
    let res = ctx
        .client
        .get(
            ctx.endpoint
                .join(&format!(
                    "/api/v0/repositories/{}/distributions/{}",
                    percent_encode(cmd.repo.as_bytes(), PATH_SEGMENT_PERCENT_ENCODE_SET),
                    percent_encode(cmd.name.as_bytes(), PATH_SEGMENT_PERCENT_ENCODE_SET)
                ))
                .unwrap(),
        )
        .send()
        .await
        .expect("Could not send API request");
    match res.status() {
        StatusCode::OK => {
            let repo = res
                .json::<ResyncRepositoryResponse>()
                .await
                .expect("Could not parse response");
            todo!()
        }
        _ => {
            let error = res
                .json::<ErrorResponse>()
                .await
                .expect("Could not parse error response");
            Err(format!("error resyncing distribution: {}", error.message))
        }
    }
}

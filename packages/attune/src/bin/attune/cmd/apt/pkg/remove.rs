use std::process::ExitCode;

use clap::Args;
use tracing::info;

use crate::config::Config;
use attune::{
    api::ErrorResponse,
    cli::{
        apt::pkg::remove::{CmdAptPkgRemove, remove_package},
        retry_delay_default, retry_infinite,
    },
};

#[derive(Args, Debug)]
pub struct PkgRemoveCommand {
    /// Name of the repository to remove the package from
    #[arg(long, short)]
    repo: String,
    /// Distribution to remove the package from
    #[arg(long, short)]
    distribution: String,
    /// Component to remove the package from
    #[arg(long, short)]
    component: String,

    /// GPG key ID to sign the index with (see `gpg --list-secret-keys`)
    #[arg(long, short)]
    key_id: String,

    /// Name of the package to remove
    #[arg(long, short)]
    package: String,
    /// Version of the package to remove
    #[arg(long, short)]
    version: String,
    /// Architecture of the package to remove
    #[arg(long, short)]
    architecture: String,
}

pub async fn run(ctx: Config, command: CmdAptPkgRemove) -> ExitCode {
    let ctx = ctx.into();
    let res = retry_infinite(
        || remove_package(&ctx, &command),
        |error| match error.downcast_ref::<ErrorResponse>() {
            Some(res) => match res.error.as_str() {
                "CONCURRENT_INDEX_CHANGE" | "DETACHED_SIGNATURE_VERIFICATION_FAILED" => {
                    tracing::warn!(error = ?res, "retrying: concurrent index change");
                    true
                }
                _ => false,
            },
            None => false,
        },
        retry_delay_default,
    )
    .await;

    match res {
        Ok(_) => {
            info!(?command.package, "package removed from index");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Error removing package from index: {error:#?}");
            ExitCode::FAILURE
        }
    }
}

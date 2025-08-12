use std::process::ExitCode;

use clap::Args;
use color_eyre::eyre::{Context as _, Result, bail};
use http::StatusCode;
use percent_encoding::percent_encode;
use tracing::{debug, info, instrument};

use attune::{
    api::{ErrorResponse, PATH_SEGMENT_PERCENT_ENCODE_SET},
    server::repo::index::{
        PackageChange, PackageChangeAction,
        generate::{GenerateIndexRequest, GenerateIndexResponse},
        sign::{SignIndexRequest, SignIndexResponse},
    },
};

use crate::{config::Config, gpg_sign, retry_delay_default, retry_infinite};

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

    /// GPG home directory to use for signing.
    ///
    /// If not set, defaults to the standard GPG home directory
    /// for the platform.
    #[arg(long, short)]
    gpg_home_dir: Option<String>,

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

pub async fn run(ctx: Config, command: PkgRemoveCommand) -> ExitCode {
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

#[instrument]
pub async fn remove_package(ctx: &Config, command: &PkgRemoveCommand) -> Result<()> {
    debug!("removing package from index");
    let generate_index_request = GenerateIndexRequest {
        change: PackageChange {
            repository: command.repo.clone(),
            distribution: command.distribution.clone(),
            component: command.component.clone(),
            action: PackageChangeAction::Remove {
                name: command.package.clone(),
                version: command.version.clone(),
                architecture: command.architecture.clone(),
            },
        },
    };
    let res = ctx
        .client
        .get(
            ctx.endpoint
                .join(
                    format!(
                        "/api/v0/repositories/{}/index",
                        percent_encode(command.repo.as_bytes(), PATH_SEGMENT_PERCENT_ENCODE_SET)
                    )
                    .as_str(),
                )
                .context("join endpoint")?,
        )
        .json(&generate_index_request)
        .send()
        .await
        .context("send API request")?;
    let (index, release_ts) = match res.status() {
        StatusCode::OK => {
            let res = res
                .json::<GenerateIndexResponse>()
                .await
                .context("parse response")?;
            debug!(index = ?res.release, "generated index to sign");
            (res.release, res.release_ts)
        }
        status => {
            let body = res.text().await.context("read response")?;
            debug!(?body, ?status, "error response");
            let error =
                serde_json::from_str::<ErrorResponse>(&body).context("parse error response")?;
            bail!(error);
        }
    };

    // Sign index locally.
    let sig = gpg_sign(command.gpg_home_dir.as_deref(), &command.key_id, index)
        .await
        .context("sign index")?;

    // Submit signatures.
    debug!("submitting signatures");
    let res = ctx
        .client
        .post(
            ctx.endpoint
                .join(
                    format!(
                        "/api/v0/repositories/{}/index",
                        percent_encode(command.repo.as_bytes(), PATH_SEGMENT_PERCENT_ENCODE_SET)
                    )
                    .as_str(),
                )
                .unwrap(),
        )
        .json(&SignIndexRequest {
            change: generate_index_request.change,
            release_ts,
            clearsigned: sig.clearsigned,
            detachsigned: sig.detachsigned,
            public_key_cert: sig.public_key_cert,
        })
        .send()
        .await
        .context("send API request")?;
    match res.status() {
        StatusCode::OK => {
            let _ = res
                .json::<SignIndexResponse>()
                .await
                .context("parse response")?;
            debug!("signed index");
            Ok(())
        }
        status => {
            let body = res.text().await.context("read response")?;
            debug!(?body, ?status, "error response");
            let error =
                serde_json::from_str::<ErrorResponse>(&body).context("parse error response")?;
            bail!(error);
        }
    }
}

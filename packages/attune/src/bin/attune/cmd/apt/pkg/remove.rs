use std::{iter::once, process::ExitCode};

use clap::Args;
use color_eyre::eyre::{Context as _, OptionExt, Result, bail};
use gpgme::{Context, ExportMode, Protocol};
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

use crate::{config::Config, retry_delay_default, retry_infinite};

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
    debug!(?index, "signing index");
    let mut gpg = Context::from_protocol(Protocol::OpenPgp).context("create gpg context")?;
    gpg.set_armor(true);
    let key = gpg
        .find_secret_keys(vec![command.key_id.as_str()])
        .context("find secret key")?
        .next()
        .ok_or_eyre("find secret key")?
        .context("find secret key")?;
    gpg.add_signer(&key).context("add signer")?;
    // TODO: Configure passphrase provider?

    let mut clearsigned = Vec::new();
    gpg.sign_clear(index.as_bytes(), &mut clearsigned)
        .context("clearsign index")?;
    let clearsigned =
        String::from_utf8(clearsigned).context("clearsigned index contained invalid characters")?;
    debug!(?clearsigned, "clearsigned index");
    let mut detachsigned = Vec::new();
    gpg.sign_detached(index.as_bytes(), &mut detachsigned)
        .context("detach sign index")?;
    let detachsigned = String::from_utf8(detachsigned)
        .context("detachsigned index contained invalid characters")?;
    debug!(?detachsigned, "detachsigned index");

    let mut public_key_cert = Vec::new();
    gpg.export_keys(once(&key), ExportMode::empty(), &mut public_key_cert)
        .context("export key")?;
    let public_key_cert = String::from_utf8(public_key_cert)
        .context("public key cert contained invalid characters")?;
    debug!(?public_key_cert, "public key cert");

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
            clearsigned,
            detachsigned,
            public_key_cert,
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

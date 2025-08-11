//! Add an APT package to the Attune server.

use std::iter::once;

use clap::Args;
use color_eyre::eyre::{Context as _, OptionExt, Result, bail};
use gpgme::{Context, ExportMode, Protocol};
use http::StatusCode;
use percent_encoding::percent_encode;
use reqwest::multipart::{self, Part};
use sha2::{Digest, Sha256};
use tracing::{Span, debug, instrument};

use crate::{
    api::{ErrorResponse, PATH_SEGMENT_PERCENT_ENCODE_SET},
    cli::Config,
    server::{
        pkg::{info::PackageInfoResponse, upload::PackageUploadResponse},
        repo::{
            index::{
                PackageChange, PackageChangeAction,
                generate::{GenerateIndexRequest, GenerateIndexResponse},
                sign::{SignIndexRequest, SignIndexResponse},
            },
            info::RepositoryInfoResponse,
        },
    },
};

#[derive(Args, Debug)]
pub struct CmdAptPkgAdd {
    /// Name of the repository to add the package to
    #[arg(long, short)]
    pub repo: String,

    /// Distribution to add the package to
    #[arg(long, short, default_value = "stable")]
    pub distribution: String,

    /// Component to add the package to
    #[arg(long, short, default_value = "main")]
    pub component: String,

    /// GPG key ID to sign the index with (see `gpg --list-secret-keys`)
    #[arg(long, short)]
    pub key_id: String,

    // TODO(#48): Implement.
    // /// Overwrite existing package, even if different
    // #[arg(long, short)]
    // overwrite: bool,
    /// Path to the package to add
    pub package_file: String,
}

/// Ensure that the specified repository exists.
#[instrument]
pub async fn validate_repository_exists(ctx: &Config, cmd: &CmdAptPkgAdd) -> Result<bool> {
    debug!("checking whether repository exists");
    let res = ctx
        .client
        .get(
            ctx.endpoint
                .join(
                    format!(
                        "/api/v0/repositories/{}",
                        percent_encode(cmd.repo.as_bytes(), PATH_SEGMENT_PERCENT_ENCODE_SET)
                    )
                    .as_str(),
                )
                .unwrap(),
        )
        .send()
        .await
        .context("send api request")?;
    match res.status() {
        StatusCode::OK => {
            let repo = res
                .json::<RepositoryInfoResponse>()
                .await
                .context("parse response")?;
            debug!(?repo, "repository exists");
            Ok(true)
        }
        StatusCode::NOT_FOUND => {
            debug!("repository does not exist");
            Ok(false)
        }
        status => {
            let body = res.text().await.context("read response")?;
            debug!(?body, ?status, "error response");
            let error =
                serde_json::from_str::<ErrorResponse>(&body).context("parse error response")?;
            bail!(error,);
        }
    }
}

/// Checksum the package file, and upload if needed.
// TODO: We might want to make this streaming for sufficiently large package
// files (ones that don't fit in memory). For small ones, I think keeping
// the file in memory might be faster.
//
// TODO: We may also want to check whether a package with the same
// identifiers (i.e. (name, version, architecture)) already exists, which
// should be impossible, and should be an error we report to the user.
//
// TODO(#48): Add an `--overwrite` flag to allow the user to deliberately upload
// a package with a different SHA256sum.
#[instrument(fields(sha256sum))]
pub async fn upsert_file_content(ctx: &Config, cmd: &CmdAptPkgAdd) -> Result<String> {
    debug!("upserting file content");

    debug!("calculating SHA256 sum");
    let content = std::fs::read(&cmd.package_file).context("read package file")?;
    let sha256sum = hex::encode(Sha256::digest(&content).as_slice());
    Span::current().record("sha256sum", &sha256sum);
    debug!("calculated SHA256 sum");

    let res = ctx
        .client
        .get(
            ctx.endpoint
                .join(format!("/api/v0/packages/{sha256sum}").as_str())
                .unwrap(),
        )
        .send()
        .await
        .context("send api request")?;

    match res.status() {
        StatusCode::OK => {
            let pkg = res
                .json::<PackageInfoResponse>()
                .await
                .context("parse response")?;
            debug!(?sha256sum, ?pkg, "package already exists, skipping upload");
            Ok(sha256sum)
        }
        StatusCode::NOT_FOUND => {
            debug!(?sha256sum, "package does not exist, uploading");
            let multipart = multipart::Form::new().part("file", Part::bytes(content));

            let res = ctx
                .client
                .post(ctx.endpoint.join("/api/v0/packages").unwrap())
                .multipart(multipart)
                .send()
                .await
                .context("send api request")?;
            match res.status() {
                StatusCode::OK => {
                    let uploaded = res
                        .json::<PackageUploadResponse>()
                        .await
                        .context("parse response")?;
                    debug!(?sha256sum, ?uploaded, "package uploaded");
                    Ok(sha256sum)
                }
                _ => {
                    let error = res
                        .json::<ErrorResponse>()
                        .await
                        .context("parse error response")?;
                    bail!(error);
                }
            }
        }
        _ => {
            let error = res
                .json::<ErrorResponse>()
                .await
                .context("parse error response")?;
            bail!(error);
        }
    }
}

/// Generate an index for the package, and sign it.
#[instrument]
pub async fn add_package(ctx: &Config, command: &CmdAptPkgAdd, sha256sum: &str) -> Result<()> {
    debug!(?sha256sum, repo = ?command.repo, distribution = ?command.distribution, component = ?command.component, "adding package to index");
    let generate_index_request = GenerateIndexRequest {
        change: PackageChange {
            repository: command.repo.clone(),
            distribution: command.distribution.clone(),
            component: command.component.clone(),
            action: PackageChangeAction::Add {
                package_sha256sum: sha256sum.to_string(),
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
                .unwrap(),
        )
        .json(&generate_index_request)
        .send()
        .await
        .context("send api request")?;
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
    debug!(?key, "using public key");
    gpg.add_signer(&key).context("add signer")?;
    // TODO: Configure passphrase provider?

    let mut clearsigned = Vec::new();
    gpg.sign_clear(index.as_bytes(), &mut clearsigned)
        .context("clearsign index")?;
    let clearsigned =
        String::from_utf8(clearsigned).context("clearsigned index contained invalid characters")?;
    debug!(?index, ?clearsigned, "clearsigned index");
    let mut detachsigned = Vec::new();
    gpg.sign_detached(index.as_bytes(), &mut detachsigned)
        .context("detach sign index")?;
    let detachsigned = String::from_utf8(detachsigned)
        .context("detachsigned index contained invalid characters")?;
    debug!(?index, ?detachsigned, "detachsigned index");

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
        .context("send api request")?;
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

use std::{iter::once, process::ExitCode};

use axum::http::StatusCode;
use clap::Args;
use gpgme::{Context, ExportMode, Protocol};
use percent_encoding::percent_encode;
use reqwest::multipart::{self, Part};
use sha2::{Digest as _, Sha256};
use tracing::debug;

use crate::{
    config::Config,
    http::{NoBody, ResponseDropStatus, ResponseRequiresBody},
};
use attune::{
    api::{ErrorResponse, PATH_SEGMENT_PERCENT_ENCODE_SET},
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
pub struct PkgAddCommand {
    /// Name of the repository to add the package to
    #[arg(long, short)]
    repo: String,
    /// Distribution to add the package to
    #[arg(long, short, default_value = "stable")]
    distribution: String,
    /// Component to add the package to
    #[arg(long, short, default_value = "main")]
    component: String,

    /// GPG key ID to sign the index with (see `gpg --list-secret-keys`)
    #[arg(long, short)]
    key_id: String,

    // TODO(#48): Implement.
    // /// Overwrite existing package, even if different
    // #[arg(long, short)]
    // overwrite: bool,
    /// Path to the package to add
    package_file: String,
}

pub async fn run(ctx: Config, command: PkgAddCommand) -> ExitCode {
    // Ensure that the specified repository exists.
    //
    // We don't technically _need_ to do this check, since it occurs in the
    // index signing steps. However, doing it here lets us short-circuit a bad
    // upload early, before we have to do the long step of actually performing
    // the upload.
    debug!(repo = ?command.repo, "checking whether repository exists");
    let res = ctx
        .client
        .get(
            ctx.endpoint
                .join(
                    format!(
                        "/api/v0/repositories/{}",
                        percent_encode(command.repo.as_bytes(), PATH_SEGMENT_PERCENT_ENCODE_SET)
                    )
                    .as_str(),
                )
                .unwrap(),
        )
        .send()
        .await
        .expect("Could not send API request");
    match res.status() {
        StatusCode::OK => {
            let repo = res
                .json::<RepositoryInfoResponse>()
                .await
                .expect("Could not parse response");
            debug!(?repo, "repository exists");
        }
        StatusCode::NOT_FOUND => {
            eprintln!("Error: repository {:?} does not exist", command.repo);
            return ExitCode::FAILURE;
        }
        status => {
            let body = res.text().await.expect("Could not read response");
            debug!(?body, ?status, "error response");
            let error = serde_json::from_str::<ErrorResponse>(&body)
                .expect("Could not parse error response");
            eprintln!(
                "Error checking whether repository exists: {}",
                error.message
            );
            return ExitCode::FAILURE;
        }
    }

    // Checksum the package file, and upload if needed.
    //
    // TODO: We might want to make this streaming for sufficiently large package
    // files (ones that don't fit in memory). For small ones, I think keeping
    // the file in memory might be faster.
    debug!(package_file = ?command.package_file, "calculating SHA256 sum");
    let package_file = std::fs::read(command.package_file).unwrap();
    let sha256sum = hex::encode(Sha256::digest(&package_file).as_slice());
    debug!(sha256sum = ?sha256sum, "calculated SHA256 sum");

    // TODO: We may also want to check whether a package with the same
    // identifiers (i.e. (name, version, architecture)) already exists, which
    // should be impossible, and should be an error we report to the user.
    //
    // TODO(#48): Add an `--overwrite` flag to allow the user to deliberately upload
    // a package with a different SHA256sum.

    debug!(?sha256sum, "checking whether package exists");
    let url = format!("/api/v0/packages/{sha256sum}");
    let res = crate::http::get::<PackageInfoResponse, _>(&ctx, &url, &NoBody)
        .await
        .drop_status();
    match res {
        Ok(Some(pkg)) => debug!(?sha256sum, ?pkg, "package already exists, skipping upload"),
        Ok(None) => {
            debug!(?sha256sum, "package does not exist, uploading");

            let body = || multipart::Form::new().part("file", Part::bytes(package_file.clone()));
            let upload = crate::http::post_multipart::<PackageUploadResponse>(&ctx, &url, body)
                .await
                .require_body()
                .drop_status();
            match upload {
                Ok(uploaded) => debug!(?sha256sum, ?uploaded, "package uploaded"),
                Err(error) => {
                    eprintln!("Error uploading package: {}", error.message);
                    return ExitCode::FAILURE;
                }
            }
        }
        Err(error) => {
            eprintln!("Error checking whether package exists: {}", error.message);
            return ExitCode::FAILURE;
        }
    }

    // TODO: Check whether the package needs to be added to the index. If the
    // package already exists in the (release, distribution, component), we can
    // skip re-signing.

    // Add the package to the index, retrying if needed.
    debug!(?sha256sum, repo = ?command.repo, distribution = ?command.distribution, component = ?command.component, "adding package to index");
    let generate_index_request = GenerateIndexRequest {
        change: PackageChange {
            repository: command.repo.clone(),
            distribution: command.distribution.clone(),
            component: command.component.clone(),
            action: PackageChangeAction::Add {
                package_sha256sum: sha256sum.clone(),
            },
        },
    };
    let res = crate::http::get::<GenerateIndexResponse, _>(
        &ctx,
        format!(
            "/api/v0/repositories/{}/index",
            percent_encode(command.repo.as_bytes(), PATH_SEGMENT_PERCENT_ENCODE_SET)
        ),
        &generate_index_request,
    )
    .await
    .require_body()
    .drop_status();
    let (index, release_ts) = match res {
        Ok(index) => {
            debug!(?index, "generated index to sign");
            (index.release, index.release_ts)
        }
        Err(error) => {
            eprintln!("Error adding package to index: {}", error.message);
            return ExitCode::FAILURE;
        }
    };

    // Sign index locally.
    debug!(?index, "signing index");
    let mut gpg = Context::from_protocol(Protocol::OpenPgp).expect("could not create gpg context");
    gpg.set_armor(true);
    let key = gpg
        .find_secret_keys(vec![command.key_id.as_str()])
        .expect("could not find secret key")
        .next()
        .expect("could not find secret key")
        .expect("could not find secret key");
    debug!(?key, "using public key");
    gpg.add_signer(&key).expect("could not add signer");
    // TODO: Configure passphrase provider?

    let mut clearsigned = Vec::new();
    gpg.sign_clear(index.as_bytes(), &mut clearsigned)
        .expect("could not clearsign index");
    let clearsigned =
        String::from_utf8(clearsigned).expect("clearsigned index contained invalid characters");
    debug!(?index, ?clearsigned, "clearsigned index");
    let mut detachsigned = Vec::new();
    gpg.sign_detached(index.as_bytes(), &mut detachsigned)
        .expect("could not detach sign index");
    let detachsigned =
        String::from_utf8(detachsigned).expect("detachsigned index contained invalid characters");
    debug!(?index, ?detachsigned, "detachsigned index");

    let mut public_key_cert = Vec::new();
    gpg.export_keys(once(&key), ExportMode::empty(), &mut public_key_cert)
        .expect("could not export key");
    let public_key_cert =
        String::from_utf8(public_key_cert).expect("public key cert contained invalid characters");
    debug!(?public_key_cert, "public key cert");

    // Submit signatures.
    debug!("submitting signatures");
    let res = crate::http::post::<SignIndexResponse, _>(
        &ctx,
        format!(
            "/api/v0/repositories/{}/index",
            percent_encode(command.repo.as_bytes(), PATH_SEGMENT_PERCENT_ENCODE_SET)
        ),
        &SignIndexRequest {
            change: generate_index_request.change,
            release_ts,
            clearsigned,
            detachsigned,
            public_key_cert,
        },
    )
    .await
    .require_body()
    .drop_status();
    match res {
        Ok(signed) => {
            debug!(?signed, "signed index");
            ExitCode::SUCCESS
        }
        Err(body) => {
            match body.error.as_str() {
                "INVALID_COMPONENT_NAME" => {
                    eprintln!(
                        "Error: Invalid component name {:?}: {}\nComponent names must contain only letters, numbers, underscores, and hyphens.",
                        command.component, body.message
                    );
                }
                _ => {
                    eprintln!("Error signing index: {}", body.message);
                }
            }
            ExitCode::FAILURE
        }
    }
}

use std::{iter::once, process::ExitCode};

use axum::http::StatusCode;
use clap::Args;
use gpgme::{Context, ExportMode, Protocol};
use percent_encoding::percent_encode;
use tracing::debug;

use crate::config::Config;
use attune::{
    api::{ErrorResponse, PATH_SEGMENT_PERCENT_ENCODE_SET},
    server::repo::index::{
        PackageChange, PackageChangeAction,
        generate::{GenerateIndexRequest, GenerateIndexResponse},
        sign::{SignIndexRequest, SignIndexResponse},
    },
};

#[derive(Args)]
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
    // Remove the package from the index, retrying if needed.
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
                .unwrap(),
        )
        .json(&generate_index_request)
        .send()
        .await
        .expect("Could not send API request");
    let (index, release_ts) = match res.status() {
        StatusCode::OK => {
            let res = res
                .json::<GenerateIndexResponse>()
                .await
                .expect("Could not parse response");
            debug!(index = ?res.release, "generated index to sign");
            (res.release, res.release_ts)
        }
        status => {
            let body = res.text().await.expect("Could not read response");
            debug!(?body, ?status, "error response");
            let error = serde_json::from_str::<ErrorResponse>(&body)
                .expect("Could not parse error response");
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
    gpg.add_signer(&key).expect("could not add signer");
    // TODO: Configure passphrase provider?

    let mut clearsigned = Vec::new();
    gpg.sign_clear(index.as_bytes(), &mut clearsigned)
        .expect("could not clearsign index");
    let clearsigned =
        String::from_utf8(clearsigned).expect("clearsigned index contained invalid characters");
    debug!(?clearsigned, "clearsigned index");
    let mut detachsigned = Vec::new();
    gpg.sign_detached(index.as_bytes(), &mut detachsigned)
        .expect("could not detach sign index");
    let detachsigned =
        String::from_utf8(detachsigned).expect("detachsigned index contained invalid characters");
    debug!(?detachsigned, "detachsigned index");

    let mut public_key_cert = Vec::new();
    gpg.export_keys(once(&key), ExportMode::empty(), &mut public_key_cert)
        .expect("could not export key");
    let public_key_cert =
        String::from_utf8(public_key_cert).expect("public key cert contained invalid characters");
    debug!(?public_key_cert, "public key cert");

    // Submit signatures.
    //
    // TODO: Implement retries on conflict.
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
        .expect("Could not send API request");
    match res.status() {
        StatusCode::OK => {
            let _ = res
                .json::<SignIndexResponse>()
                .await
                .expect("Could not parse response");
            debug!("signed index");
            ExitCode::SUCCESS
        }
        // TODO: Handle 409 status code to signal retry.
        status => {
            let body = res.text().await.expect("Could not read response");
            debug!(?body, ?status, "error response");
            let error = serde_json::from_str::<ErrorResponse>(&body)
                .expect("Could not parse error response");
            eprintln!("Error signing index: {}", error.message);
            ExitCode::FAILURE
        }
    }
}

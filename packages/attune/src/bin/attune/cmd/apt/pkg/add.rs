use std::{
    io::{Read, Write, pipe},
    process::{Command, ExitCode, Stdio},
};

use axum::http::StatusCode;
use clap::Args;
use percent_encoding::percent_encode;
use reqwest::multipart::{self, Part};
use sha2::{Digest as _, Sha256};
use tracing::{debug, trace};

use crate::config::Config;
use attune::{
    api::{ErrorResponse, PATH_SEGMENT_PERCENT_ENCODE_SET},
    server::{
        pkg::{info::PackageInfoResponse, upload::PackageUploadResponse},
        repo::{
            index::generate::{GenerateIndexRequest, GenerateIndexResponse, IndexChange},
            info::RepositoryInfoResponse,
        },
    },
};

#[derive(Args)]
pub struct PkgAddCommand {
    /// Name of the repository to add the package to
    #[arg(long, short)]
    repo: String,
    /// Distribution to add the package to
    #[arg(long, short)]
    distribution: String,
    /// Component to add the package to
    #[arg(long, short)]
    component: String,

    /// GPG key ID to sign the index with (see `gpg --list-secret-keys`)
    #[arg(long, short)]
    key_id: String,

    // TODO: Implement.
    // /// Overwrite existing package, even if different
    // #[arg(long, short)]
    // overwrite: bool,
    /// Path to the package to add
    package_file: String,
}

pub async fn run(ctx: Config, command: PkgAddCommand) -> ExitCode {
    // Ensure that the specified repository exists.
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
    let sha256sum = hex::encode(Sha256::digest(&package_file).as_slice().to_vec());
    debug!(sha256sum = ?sha256sum, "calculated SHA256 sum");

    debug!(sha256sum = ?sha256sum, "checking whether package exists");
    let res = ctx
        .client
        .get(
            ctx.endpoint
                .join(format!("/api/v0/packages/{}", sha256sum).as_str())
                .unwrap(),
        )
        .send()
        .await
        .expect("Could not send API request");
    match res.status() {
        StatusCode::OK => {
            let pkg = res
                .json::<PackageInfoResponse>()
                .await
                .expect("Could not parse response");
            debug!(?sha256sum, ?pkg, "package already exists, skipping upload");
        }
        StatusCode::NOT_FOUND => {
            debug!(sha256sum = ?sha256sum, "package does not exist, uploading");
            let multipart = multipart::Form::new().part("file", Part::bytes(package_file));

            let res = ctx
                .client
                .post(ctx.endpoint.join("/api/v0/packages").unwrap())
                .multipart(multipart)
                .send()
                .await
                .expect("Could not upload package file");
            match res.status() {
                StatusCode::OK => {
                    let uploaded = res
                        .json::<PackageUploadResponse>()
                        .await
                        .expect("Could not parse response");
                    debug!(?sha256sum, ?uploaded, "package uploaded");
                }
                _ => {
                    let error = res
                        .json::<ErrorResponse>()
                        .await
                        .expect("Could not parse error response");
                    eprintln!("Error uploading package: {}", error.message);
                    return ExitCode::FAILURE;
                }
            }
        }
        _ => {
            let error = res
                .json::<ErrorResponse>()
                .await
                .expect("Could not parse error response");
            eprintln!("Error checking whether package exists: {}", error.message);
            return ExitCode::FAILURE;
        }
    }

    // Add the package to the index, retrying if needed.
    debug!(?sha256sum, repo = ?command.repo, distribution = ?command.distribution, component = ?command.component, "adding package to index");
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
        .json(&GenerateIndexRequest {
            repository: command.repo,
            distribution: command.distribution,
            component: command.component,
            package_sha256sum: sha256sum,
            change: IndexChange::Add,
        })
        .send()
        .await
        .expect("Could not send API request");
    let index = match res.status() {
        StatusCode::OK => {
            let res = res
                .json::<GenerateIndexResponse>()
                .await
                .expect("Could not parse response");
            debug!(index = ?res.release, "generated index to sign");
            res.release
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

    // Sign package locally.
    debug!(?index, "clearsigning index");
    let (reader, mut writer) = pipe().expect("could not create pipe");
    trace!("spawning gpg");
    let mut clearsigner = Command::new("gpg")
        .arg("--clearsign")
        .arg("--local-user")
        .arg(&command.key_id)
        .arg("--batch")
        .arg("--yes")
        .stdin(reader)
        .stdout(Stdio::piped())
        .spawn()
        .expect("could not spawn gpg command");
    trace!("gpg spawned");
    writer
        .write_all(index.as_bytes())
        .expect("could not write to pipe");
    writer.flush().expect("could not flush pipe");
    trace!("wrote index to pipe");
    drop(writer);
    let status = clearsigner.wait().expect("gpg failed to finish");
    if !status.success() {
        eprintln!("Error clearsigning index: {}", status);
        return ExitCode::FAILURE;
    }
    let mut clearsigned = String::new();
    clearsigner
        .stdout
        .unwrap()
        .read_to_string(&mut clearsigned)
        .expect("could not read clearsigned index");
    debug!(?clearsigned, "clearsigned index");

    debug!(?index, "detachsigning index");
    let (reader, mut writer) = pipe().expect("could not create pipe");
    trace!("spawning gpg");
    let mut detachsigner = Command::new("gpg")
        .arg("--detach-sign")
        .arg("--armor")
        .arg("--local-user")
        .arg(&command.key_id)
        .arg("--batch")
        .arg("--yes")
        .stdin(reader)
        .stdout(Stdio::piped())
        .spawn()
        .expect("could not spawn gpg command");
    trace!("gpg spawned");
    writer
        .write_all(index.as_bytes())
        .expect("could not write to pipe");
    writer.flush().expect("could not flush pipe");
    trace!("wrote index to pipe");
    drop(writer);
    let status = detachsigner.wait().expect("gpg failed to finish");
    if !status.success() {
        eprintln!("Error detachsigning index: {}", status);
        return ExitCode::FAILURE;
    }
    let mut detachsigned = String::new();
    detachsigner
        .stdout
        .unwrap()
        .read_to_string(&mut detachsigned)
        .expect("could not read detachsigned index");
    debug!(?detachsigned, "detachsigned index");

    // Submit signatures.

    todo!()
}

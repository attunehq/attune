use std::process::ExitCode;

use crate::{config::Config, gpg_sign, retry_delay_default, retry_infinite};

use bon::Builder;
use clap::Args;
use color_eyre::eyre::{Context as _, Result, bail};
use http::StatusCode;
use percent_encoding::percent_encode;
use reqwest::multipart::{self, Part};
use sha2::{Digest, Sha256};
use tracing::{Span, debug, instrument};

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

#[derive(Args, Debug, Builder, Clone)]
pub struct PkgAddCommand {
    /// Name of the repository to add the package to
    #[arg(long, short)]
    #[builder(into)]
    pub repo: String,

    /// Distribution to add the package to
    #[arg(long, short, default_value = "stable")]
    #[builder(into)]
    pub distribution: String,

    /// Component to add the package to
    #[arg(long, short, default_value = "main")]
    #[builder(into)]
    pub component: String,

    /// GPG key ID to sign the index with (see `gpg --list-secret-keys`)
    #[arg(long, short)]
    #[builder(into)]
    pub key_id: String,

    /// GPG home directory to use for signing.
    ///
    /// If not set, defaults to the standard GPG home directory
    /// for the platform.
    #[arg(long, short)]
    #[builder(into)]
    pub gpg_home_dir: Option<String>,

    // TODO(#48): Implement.
    // /// Overwrite existing package, even if different
    // #[arg(long, short)]
    // overwrite: bool,
    /// Path to the package to add
    #[builder(into)]
    pub package_file: String,
}

pub async fn run(ctx: Config, command: PkgAddCommand) -> ExitCode {
    match validate_repository_exists(&ctx, &command).await {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("Error: repository {:?} does not exist", command.repo);
            return ExitCode::FAILURE;
        }
        Err(error) => {
            eprintln!("Unable to validate repository: {error:#?}");
            return ExitCode::FAILURE;
        }
    }

    let sha256sum = match upsert_file_content(&ctx, &command).await {
        Ok(sha256sum) => sha256sum,
        Err(error) => {
            eprintln!("Unable to upsert file content: {error:#?}");
            return ExitCode::FAILURE;
        }
    };

    // TODO: Check whether the package needs to be added to the index. If the
    // package already exists in the (release, distribution, component), we can
    // skip re-signing.

    // Add the package to the index, retrying if needed.
    let res = retry_infinite(
        || add_package(&ctx, &command, &sha256sum),
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
            tracing::info!(?sha256sum, "package added to index");
            ExitCode::SUCCESS
        }
        Err(error) => match error.downcast::<ErrorResponse>() {
            Ok(res) => match res.error.as_str() {
                "INVALID_COMPONENT_NAME" => {
                    eprintln!(
                        "Error: Invalid component name {:?}: {}\nComponent names must contain only letters, numbers, underscores, and hyphens.",
                        command.component, res.message
                    );
                    ExitCode::FAILURE
                }
                _ => {
                    eprintln!("Unable to add package to index: {}", res.message);
                    ExitCode::FAILURE
                }
            },
            Err(other) => {
                eprintln!("Unable to add package to index: {other:#?}");
                ExitCode::FAILURE
            }
        },
    }
}

/// Ensure that the specified repository exists.
#[instrument]
pub async fn validate_repository_exists(ctx: &Config, cmd: &PkgAddCommand) -> Result<bool> {
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
pub async fn upsert_file_content(ctx: &Config, cmd: &PkgAddCommand) -> Result<String> {
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
pub async fn add_package(ctx: &Config, command: &PkgAddCommand, sha256sum: &str) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use std::fs::read_dir;

    use attune::testing::{
        AttuneTestServer, AttuneTestServerConfig, MIGRATOR, emphemeral_gpg_key_id,
    };
    use workspace_root::get_workspace_root;

    use super::*;

    #[test_log::test(sqlx::test(migrator = "MIGRATOR"))]
    async fn abort_on_concurrent_index_change(pool: sqlx::PgPool) {
        let (key_id, _gpg, gpg_home_dir) = emphemeral_gpg_key_id()
            .await
            .expect("failed to create GPG key");

        let server = AttuneTestServer::new(AttuneTestServerConfig {
            db: pool,
            s3_bucket_name: None,
            http_api_token: None,
        })
        .await;

        const REPO_NAME: &str = "abort_on_concurrent_index_change";
        let (tenant_id, api_token) = server.create_test_tenant(REPO_NAME).await;
        server.create_repository(tenant_id, REPO_NAME).await;

        let fixtures_dir = get_workspace_root().join("scripts/fixtures");
        let fixtures = read_dir(&fixtures_dir)
            .unwrap_or_else(|err| panic!("failed to read fixtures at {fixtures_dir:?}: {err:#?}"))
            .filter_map(|entry| {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_file() { Some(path) } else { None }
            })
            .collect::<Vec<_>>();

        let ctx = Config::new(api_token, server.base_url);
        let set = fixtures
            .into_iter()
            .fold(tokio::task::JoinSet::new(), |mut set, fixture| {
                let ctx = ctx.clone();
                let gpg_home_dir = gpg_home_dir.dir_path().to_string_lossy().to_string();
                let command = PkgAddCommand::builder()
                    .repo(REPO_NAME)
                    .distribution("test")
                    .component("test")
                    .key_id(&key_id)
                    .gpg_home_dir(gpg_home_dir)
                    .package_file(fixture.to_string_lossy())
                    .build();
                set.spawn(async move {
                    let sha = upsert_file_content(&ctx, &command).await?;
                    add_package(&ctx, &command, &sha).await
                });
                set
            });

        // Since we aren't retrying these, we expect at least one to fail.
        let (failures, successes) = set
            .join_all()
            .await
            .into_iter()
            .inspect(|res| debug!("join result: {res:#?}"))
            .fold(
                (Vec::new(), Vec::new()),
                |(mut failures, mut successes), res| {
                    match res {
                        Ok(res) => successes.push(res),
                        Err(res) => failures.push(res),
                    }
                    (failures, successes)
                },
            );
        assert!(!failures.is_empty(), "at least one failure expected");
        assert!(!successes.is_empty(), "at least one success expected");
        assert!(
            failures.iter().any(|error| {
                error.downcast_ref::<ErrorResponse>().is_some_and(|res| {
                    res.error == "CONCURRENT_INDEX_CHANGE"
                        || res.error == "DETACHED_SIGNATURE_VERIFICATION_FAILED"
                })
            }),
            "at least one concurrent index change or detached signature verification error expected",
        );
    }
}

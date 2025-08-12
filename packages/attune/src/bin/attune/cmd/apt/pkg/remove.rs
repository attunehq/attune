use std::process::ExitCode;

use bon::Builder;
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

#[derive(Args, Debug, Builder)]
pub struct PkgRemoveCommand {
    /// Name of the repository to remove the package from
    #[arg(long, short)]
    #[builder(into)]
    repo: String,
    /// Distribution to remove the package from
    #[arg(long, short)]
    #[builder(into)]
    distribution: String,
    /// Component to remove the package from
    #[arg(long, short)]
    #[builder(into)]
    component: String,

    /// GPG key ID to sign the index with (see `gpg --list-secret-keys`)
    #[arg(long, short)]
    #[builder(into)]
    key_id: String,

    /// GPG home directory to use for signing.
    ///
    /// If not set, defaults to the standard GPG home directory
    /// for the platform.
    #[arg(long, short)]
    #[builder(into)]
    gpg_home_dir: Option<String>,

    /// Name of the package to remove
    #[arg(long, short)]
    #[builder(into)]
    package: String,
    /// Version of the package to remove
    #[arg(long, short)]
    #[builder(into)]
    version: String,
    /// Architecture of the package to remove
    #[arg(long, short)]
    #[builder(into)]
    architecture: String,
}

pub async fn run(ctx: Config, command: PkgRemoveCommand) -> ExitCode {
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

#[cfg(test)]
mod tests {
    use std::fs::read_dir;

    use attune::testing::{AttuneTestServer, AttuneTestServerConfig, MIGRATOR, gpg_key_id};
    use workspace_root::get_workspace_root;

    use super::*;
    use crate::cmd::apt::pkg::add::{PkgAddCommand, add_package, upload_file_content};
    use attune::server::pkg::list::{PackageListParams, PackageListResponse};

    #[test_log::test(sqlx::test(migrator = "MIGRATOR"))]
    async fn abort_on_concurrent_index_change(pool: sqlx::PgPool) {
        let (key_id, _gpg, gpg_home_dir) = gpg_key_id().await.expect("failed to create GPG key");

        let server = AttuneTestServer::new(AttuneTestServerConfig {
            db: pool,
            s3_bucket_name: None,
            http_api_token: None,
        })
        .await;

        const REPO_NAME: &str = "abort_on_concurrent_removal";
        let (tenant_id, api_token) = server.create_test_tenant(REPO_NAME).await;
        server.create_repository(tenant_id, REPO_NAME).await;

        let fixtures_dir = get_workspace_root().join("scripts/fixtures");
        let fixtures = read_dir(&fixtures_dir)
            .unwrap_or_else(|err| panic!("failed to read fixtures at {fixtures_dir:?}: {err:#?}"))
            .filter_map(|entry| {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("deb") {
                    Some(path)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        // The point of the test is to validate that concurrently removing
        // packages trigger the concurrent index change error;
        // in order to do that we need to add all the packages first.
        let ctx = Config::new(api_token, server.base_url);
        for fixture in &fixtures {
            let command = PkgAddCommand::builder()
                .repo(REPO_NAME)
                .distribution("test")
                .component("test")
                .key_id(&key_id)
                .gpg_home_dir(gpg_home_dir.dir_path().to_string_lossy())
                .package_file(fixture.to_string_lossy())
                .build();

            let sha = upload_file_content(&ctx, &command)
                .await
                .expect("failed to upsert file content");
            add_package(&ctx, &command, &sha)
                .await
                .expect("failed to add package");
        }

        let res = ctx
            .client
            .get(ctx.endpoint.join("/api/v0/packages").unwrap())
            .query(&PackageListParams {
                repository: Some(REPO_NAME.to_string()),
                distribution: Some("test".to_string()),
                component: Some("test".to_string()),
                name: None,
                version: None,
                architecture: None,
            })
            .send()
            .await
            .expect("failed to list packages");

        let packages = res
            .json::<PackageListResponse>()
            .await
            .expect("failed to parse package list response")
            .packages;

        // Concurrently remove all packages.
        // This is the actual point of the test.
        let set = packages
            .into_iter()
            .fold(tokio::task::JoinSet::new(), |mut set, pkg| {
                let ctx = ctx.clone();
                let key_id = key_id.clone();
                let command = PkgRemoveCommand::builder()
                    .repo(REPO_NAME)
                    .distribution("test")
                    .component("test")
                    .key_id(key_id)
                    .gpg_home_dir(gpg_home_dir.dir_path().to_string_lossy())
                    .package(pkg.name)
                    .version(pkg.version)
                    .architecture(pkg.architecture)
                    .build();
                set.spawn(async move { remove_package(&ctx, &command).await });
                set
            });

        // Since we aren't retrying these, we expect at least one to
        // fail with concurrent index change or detached signature.
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

        // Of course, if a bunch fail, at least one needs to succeed.
        // If this doesn't happen it's an issue, because users
        // won't be able to actually use the service.
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

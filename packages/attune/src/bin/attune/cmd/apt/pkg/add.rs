use std::process::ExitCode;

use crate::config::Config;
use attune::{
    api::ErrorResponse,
    cli::{
        apt::pkg::add::{
            CmdAptPkgAdd, add_package, upsert_file_content, validate_repository_exists,
        },
        retry_delay_default, retry_infinite,
    },
};

pub async fn run(ctx: Config, command: CmdAptPkgAdd) -> ExitCode {
    let ctx = ctx.into();
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
            return ExitCode::SUCCESS;
        }
        Err(error) => match error.downcast::<ErrorResponse>() {
            Ok(res) => match res.error.as_str() {
                "INVALID_COMPONENT_NAME" => {
                    eprintln!(
                        "Error: Invalid component name {:?}: {}\nComponent names must contain only letters, numbers, underscores, and hyphens.",
                        command.component, res.message
                    );
                    return ExitCode::FAILURE;
                }
                _ => {
                    eprintln!("Unable to add package to index: {}", res.message);
                    return ExitCode::FAILURE;
                }
            },
            Err(other) => {
                eprintln!("Unable to add package to index: {other:#?}");
                return ExitCode::FAILURE;
            }
        },
    }
}

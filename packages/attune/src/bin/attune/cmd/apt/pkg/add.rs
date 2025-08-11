use std::{process::ExitCode, time::Duration};

use crate::config::Config;
use attune::{
    api::ErrorResponse,
    cli::apt::pkg::add::{
        CmdAptPkgAdd, add_package, upsert_file_content, validate_repository_exists,
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
    const STATIC_RETRY_DELAY_MS: u64 = 2000;
    loop {
        match add_package(&ctx, &command, &sha256sum).await {
            Ok(_) => {
                tracing::info!(?sha256sum, "package added to index");
                return ExitCode::SUCCESS;
            }
            Err(error) => match error.downcast::<ErrorResponse>() {
                Ok(res) => match res.error.as_str() {
                    "CONCURRENT_INDEX_CHANGE" | "DETACHED_SIGNATURE_VERIFICATION_FAILED" => {
                        let delay = Duration::from_millis(
                            STATIC_RETRY_DELAY_MS + rand::random_range(0..STATIC_RETRY_DELAY_MS),
                        );
                        tracing::warn!(?delay, error = ?res, "retrying: concurrent index change");
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    "INVALID_COMPONENT_NAME" => {
                        eprintln!(
                            "Error: Invalid component name {:?}: {}\nComponent names must contain only letters, numbers, underscores, and hyphens.",
                            command.component, res.message
                        );
                        return ExitCode::FAILURE;
                    }
                    _ => {
                        eprintln!("Error adding package to index: {}", res.message);
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
}

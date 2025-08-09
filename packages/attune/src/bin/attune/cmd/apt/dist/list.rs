use std::process::ExitCode;

use clap::Args;
use tabled::settings::Style;

use crate::{
    cmd::apt::dist::build_distribution_url,
    config::Config,
};
use attune::{
    api::ErrorResponse,
    server::repo::dist::list::ListDistributionsResponse,
};

#[derive(Args, Debug)]
pub struct ListArgs {
    /// The name of the repository.
    #[arg(long)]
    repo: String,
}

pub async fn run(ctx: Config, args: ListArgs) -> ExitCode {
    loop {
        match list_distributions(&ctx, &args).await {
            Ok(message) => {
                println!("{message}");
                return ExitCode::SUCCESS;
            }
            Err(error) => if crate::retry::should_retry(&error) {
                let delay = crate::retry::calculate_retry_delay();
                tracing::warn!(?delay, ?error, "retrying: concurrent change");
                tokio::time::sleep(delay).await;
                continue;
            } else {
                eprintln!("Error listing distributions: {}", error.message);
                return ExitCode::FAILURE;
            },
        }
    }
}

async fn list_distributions(ctx: &Config, args: &ListArgs) -> Result<String, ErrorResponse> {
    let url = build_distribution_url(ctx, &args.repo, None);
    let res = ctx
        .client
        .get(url)
        .send()
        .await
        .map_err(|err| {
            ErrorResponse::builder()
                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                .error("REQUEST_FAILED")
                .message(format!("Failed to send request: {err}"))
                .build()
        })?;

    let status = res.status();
    if status == axum::http::StatusCode::OK {
        let response = res
            .json::<ListDistributionsResponse>()
            .await
            .map_err(|err| {
                ErrorResponse::builder()
                    .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                    .error("PARSE_ERROR")
                    .message(format!("Failed to parse response: {err}"))
                    .build()
            })?;

        if response.distributions.is_empty() {
            return Ok(format!(
                "No distributions found in repository {:?}",
                args.repo
            ));
        }

        let mut builder = tabled::builder::Builder::new();
        builder.push_record([
            "Name",
            "Suite",
            "Codename",
            "Description",
            "Origin",
            "Label",
            "Version",
        ]);
        for dist in response.distributions {
            builder.push_record([
                dist.distribution,
                dist.suite,
                dist.codename,
                dist.description.unwrap_or(String::from("(unset)")),
                dist.origin.unwrap_or(String::from("(unset)")),
                dist.label.unwrap_or(String::from("(unset)")),
                dist.version.unwrap_or(String::from("(unset)")),
            ]);
        }

        let mut table = builder.build();
        table.with(Style::modern());
        Ok(table.to_string())
    } else {
        let error = res
            .json::<ErrorResponse>()
            .await
            .map_err(|err| {
                ErrorResponse::builder()
                    .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                    .error("PARSE_ERROR")
                    .message(format!("Failed to parse error response: {err}"))
                    .build()
            })?;
        Err(error)
    }
}

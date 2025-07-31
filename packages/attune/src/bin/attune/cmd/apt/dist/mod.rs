use axum::http::StatusCode;
use clap::{Args, Subcommand};
use percent_encoding::percent_encode;

use crate::config::Config;
use attune::api::{ErrorResponse, PATH_SEGMENT_PERCENT_ENCODE_SET};

mod create;
mod delete;
mod edit;
mod list;

#[derive(Args)]
pub struct DistCommand {
    #[command(subcommand)]
    subcommand: DistSubCommand,
}

#[derive(Subcommand)]
pub enum DistSubCommand {
    /// Create a new distribution
    ///
    /// For details on the meanings of distribution ("Release") metadata fields,
    /// see <https://wiki.debian.org/DebianRepository/Format>.
    #[command(visible_aliases = ["new", "add"])]
    Create(create::CreateArgs),

    /// Show information about distributions
    #[command(visible_alias = "ls")]
    List(list::ListArgs),

    /// Edit distribution metadata
    ///
    /// For details on the meanings of distribution ("Release") metadata fields,
    /// see <https://wiki.debian.org/DebianRepository/Format>.
    ///
    /// Note that this will not actually update the published Release file until
    /// the next time you publish a package.
    #[command(visible_alias = "set")]
    Edit(edit::EditArgs),

    /// Delete a distribution
    #[command(visible_alias = "rm")]
    Delete(delete::DeleteArgs),
}

pub async fn handle_dist(ctx: Config, command: DistCommand) -> Result<String, String> {
    match command.subcommand {
        DistSubCommand::Create(args) => create::run(ctx, args).await,
        DistSubCommand::List(args) => list::run(ctx, args).await,
        DistSubCommand::Edit(args) => edit::run(ctx, args).await,
        DistSubCommand::Delete(args) => delete::run(ctx, args).await,
    }
}

/// Build URL for distribution API endpoints
fn build_distribution_url(
    config: &Config,
    repository: &str,
    distribution: Option<&str>,
) -> reqwest::Url {
    let path = match distribution {
        Some(dist) => format!(
            "/api/v0/repositories/{}/distributions/{}",
            percent_encode(repository.as_bytes(), PATH_SEGMENT_PERCENT_ENCODE_SET),
            percent_encode(dist.as_bytes(), PATH_SEGMENT_PERCENT_ENCODE_SET)
        ),
        None => format!(
            "/api/v0/repositories/{}/distributions",
            percent_encode(repository.as_bytes(), PATH_SEGMENT_PERCENT_ENCODE_SET)
        ),
    };
    config
        .endpoint
        .join(&path)
        .expect("Invalid URL construction")
}

/// Handle API response, accounting for the structured error type.
async fn handle_api_response<T>(response: reqwest::Response) -> Result<T, String>
where
    T: for<'de> serde::Deserialize<'de>,
{
    if response.status() == StatusCode::OK {
        response
            .json::<T>()
            .await
            .map_err(|e| format!("Failed to parse API response: {e}"))
    } else {
        response
            .json::<ErrorResponse>()
            .await
            .map(|err| Err(format!("API error: {}", err.message)))
            .map_err(|err| format!("Failed to parse error response: {err}"))?
    }
}

use std::process::ExitCode;

use clap::{Args, Subcommand};
use percent_encoding::percent_encode;

use crate::config::Config;
use attune::api::PATH_SEGMENT_PERCENT_ENCODE_SET;

mod create;
mod delete;
mod edit;
mod list;
mod resync;

#[derive(Args, Debug)]
pub struct DistCommand {
    #[command(subcommand)]
    subcommand: DistSubCommand,
}

// TODO(#103): Add a command that allows us to do a no-op re-signing of the
// index, potentially with a different key.
#[derive(Subcommand, Debug)]
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

    /// Resynchronize repository from database
    ///
    /// This is only useful for self-hosted instances. This is primarily for
    /// restoring repository state after very rare race conditions or crashes.
    Resync(resync::DistResyncCommand),
}

pub async fn handle_dist(ctx: Config, command: DistCommand) -> ExitCode {
    match command.subcommand {
        DistSubCommand::Create(args) => create::run(ctx, args).await,
        DistSubCommand::List(args) => list::run(ctx, args).await,
        DistSubCommand::Edit(args) => edit::run(ctx, args).await,
        DistSubCommand::Delete(args) => delete::run(ctx, args).await,
        DistSubCommand::Resync(args) => resync::run(ctx, args).await,
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


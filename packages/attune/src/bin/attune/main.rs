use std::{iter::once, process::ExitCode, time::Duration};

use attune::{api::ErrorResponse, server::compatibility::CompatibilityResponse};
use axum::http::StatusCode;
use clap::{Parser, Subcommand};
use color_eyre::{
    Result,
    eyre::{Context as _, OptionExt, bail},
};
use colored::Colorize;
use git_version::git_version;
use gpgme::{Context, ExportMode, Protocol};
use tracing::debug;
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

mod cmd;
mod config;

/// Attune CLI
///
/// Attune is the easiest way to securely publish Linux packages.
#[derive(Parser, Debug)]
#[command(
    name = "attune",
    version = git_version!(args = ["--tags", "--always", "--dirty=-modified"], fallback = "unknown"),
    max_term_width = 80
)]
struct Args {
    /// Attune API token.
    #[arg(long, env = "ATTUNE_API_TOKEN")]
    api_token: String,

    /// Attune API endpoint.
    #[arg(
        long,
        env = "ATTUNE_API_ENDPOINT",
        default_value = "https://api.attunehq.com"
    )]
    api_endpoint: String,

    /// Tool to run.
    #[command(subcommand)]
    tool: ToolCommand,
}

#[derive(Subcommand, Debug)]
enum ToolCommand {
    /// Manage APT repositories
    Apt(cmd::apt::AptCommand),
}

#[tokio::main]
async fn main() -> ExitCode {
    // Set up logging.
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
                .with_file(true)
                .with_line_number(true)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_writer(std::io::stderr)
                .pretty(),
        )
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    debug!(?args, "parsed arguments");

    let ctx = config::Config::new(args.api_token, args.api_endpoint);

    // Do a check for API version compatibility.
    let res = ctx
        .client
        .get(ctx.endpoint.join("/api/v0/compatibility").unwrap())
        .send()
        .await
        .expect("Could not reach API server");
    match res.status() {
        StatusCode::OK => {
            let compatibility = res
                .json::<CompatibilityResponse>()
                .await
                .expect("Could not parse compatibility response");
            match compatibility {
                CompatibilityResponse::Ok => {}
                CompatibilityResponse::WarnUpgrade { latest } => {
                    eprintln!("{} {}\n", "New version of attune available".blue(), latest);
                }
                CompatibilityResponse::Incompatible { minimum } => {
                    eprintln!(
                        "Error: CLI version is incompatible with API server. Please upgrade to version {minimum:?} or newer."
                    );
                    return ExitCode::FAILURE;
                }
            }
        }
        _ => {
            let err = res
                .json::<ErrorResponse>()
                .await
                .expect("Could not parse error response");
            eprintln!(
                "Error: could not check CLI version compatibility: {}",
                err.message
            );
            return ExitCode::FAILURE;
        }
    }

    // Execute subcommand.
    //
    // TODO: We should update all the subcommands to return `Result<String,
    // ErrorResponse>`       so that we can centralize retries, pretty printing,
    // etc.
    match args.tool {
        ToolCommand::Apt(command) => cmd::apt::handle_apt(ctx, command).await,
    }
}

/// Infinitely retry an asynchronous function call.
///
/// - `operation` is the function to call.
/// - `should_retry` evaluates whether the operation should be retried.
/// - `retry_delay` provides the duration to wait before retrying.
///
/// Optionally, you can use [`retry_delay_default`] for default delay timings.
pub async fn retry_infinite<T, E>(
    operation: impl AsyncFn() -> Result<T, E>,
    should_retry: impl Fn(&E) -> bool,
    retry_delay: impl Fn(usize) -> Duration,
) -> Result<T, E> {
    for attempt in 0usize.. {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                if should_retry(&e) {
                    tokio::time::sleep(retry_delay(attempt)).await;
                } else {
                    return Err(e);
                }
            }
        }
    }
    unreachable!("loop is functionally infinite");
}

/// The default retry delay is a static delay of 2 seconds
/// plus a random jitter of up to 2 seconds.
pub fn retry_delay_default(_: usize) -> Duration {
    const STATIC_RETRY_DELAY_MS: u64 = 2000;
    Duration::from_millis(STATIC_RETRY_DELAY_MS + rand::random_range(0..STATIC_RETRY_DELAY_MS))
}

/// The result of signing content with a GPG key.
#[derive(Debug, Clone)]
pub struct SignedGpgContent {
    pub clearsigned: String,
    pub detachsigned: String,
    pub public_key_cert: String,
}

/// Sign content with the named GPG key ID.
pub async fn gpg_sign(
    gpg_home_dir: Option<impl Into<String>>,
    key_id: Option<impl Into<String>>,
    content: impl Into<Vec<u8>>,
) -> Result<SignedGpgContent> {
    let gpg_home = gpg_home_dir.map(|p| p.into());
    let key_id = key_id.map(|k| k.into());
    let content = content.into();
    tokio::task::spawn_blocking(move || gpg_sign_blocking(gpg_home, key_id, content))
        .await
        .context("join background thread")?
}

fn gpg_sign_blocking(
    gpg_home: Option<String>,
    key_id: Option<String>,
    content: Vec<u8>,
) -> Result<SignedGpgContent> {
    let mut gpg = Context::from_protocol(Protocol::OpenPgp).context("create gpg context")?;
    if let Some(gpg_home) = gpg_home {
        gpg.set_engine_home_dir(&gpg_home)
            .with_context(|| format!("set engine home dir to: {gpg_home:?}"))?;
    }

    gpg.set_armor(true);
    let key = match key_id {
        Some(key_id) => gpg
            .find_secret_keys([&key_id])
            .context("list secret keys")?
            .next()
            .ok_or_eyre("get next key in list")?
            .context("get secret key from list")?,
        None => {
            let mut all_secret_keys = gpg
                .find_secret_keys([] as [&str; 0])
                .context("list secret keys")?
                .collect::<Result<Vec<_>, _>>()
                .context("get secret key from list")?;
            if all_secret_keys.len() == 1 {
                all_secret_keys.pop().ok_or_eyre("pop solo secret key")?
            } else {
                bail!("no GPG key ID specified and multiple GPG keys found")
            }
        }
    };
    debug!(?key, "using signing key");
    gpg.add_signer(&key).context("add signer")?;
    // TODO: Configure passphrase provider?

    let mut clearsigned = Vec::new();
    gpg.sign_clear(&content, &mut clearsigned)
        .context("clearsign index")?;
    let clearsigned =
        String::from_utf8(clearsigned).context("clearsigned index contained invalid characters")?;
    debug!(?content, ?clearsigned, "clearsigned index");
    let mut detachsigned = Vec::new();
    gpg.sign_detached(&content, &mut detachsigned)
        .context("detach sign index")?;
    let detachsigned = String::from_utf8(detachsigned)
        .context("detachsigned index contained invalid characters")?;
    debug!(?content, ?detachsigned, "detachsigned index");

    let mut public_key_cert = Vec::new();
    gpg.export_keys(once(&key), ExportMode::empty(), &mut public_key_cert)
        .context("export key")?;
    let public_key_cert = String::from_utf8(public_key_cert)
        .context("public key cert contained invalid characters")?;
    debug!(?public_key_cert, "public key cert");

    Ok(SignedGpgContent {
        clearsigned,
        detachsigned,
        public_key_cert,
    })
}

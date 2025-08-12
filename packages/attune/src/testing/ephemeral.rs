use std::path::Path;

use async_tempfile::TempDir;
use color_eyre::eyre::{Context as _, Result, eyre};
use gpgme::{Context, CreateKeyFlags, Protocol};
use tokio::process::Command;

/// Creates a new GPG key for testing in a temporary directory.
///
/// The returned directory is cleaned up when it goes out of scope.
/// The returned string is the fingerprint of the key.
pub async fn emphemeral_gpg_key_id() -> Result<(String, Context, TempDir)> {
    // Use /tmp directly to avoid socket path length issues on macOS
    let dir = TempDir::new_in(Path::new("/tmp")).await?;
    let gnupghome = dir.dir_path().to_string_lossy().to_string();

    // Create gpg-agent.conf to allow loopback pinentry
    let agent_conf_path = dir.dir_path().join("gpg-agent.conf");
    tokio::fs::write(
        &agent_conf_path,
        "allow-loopback-pinentry\n",
    )
    .await
    .context("write gpg-agent.conf")?;

    // Start gpg-agent in daemon mode (it will background itself)
    let _ = Command::new("gpg-agent")
        .env("GNUPGHOME", &gnupghome)
        .arg("--homedir")
        .arg(&gnupghome)
        .arg("--daemon")
        .arg("--allow-loopback-pinentry")
        .spawn()
        .context("spawn gpg-agent")?;
    
    // Give the agent a moment to start
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let mut gpg = Context::from_protocol(Protocol::OpenPgp).context("create gpg context")?;
    gpg.set_engine_home_dir(&gnupghome)
        .context("set engine home dir")?;
    gpg.set_armor(true);

    // Disable pinentry for batch mode operation
    gpg.set_pinentry_mode(gpgme::PinentryMode::Loopback)
        .context("set pinentry mode")?;

    let keygen_result = gpg
        .create_key_with_flags(
            "Attune Test",
            "default",
            Default::default(),
            CreateKeyFlags::NOPASSWD | CreateKeyFlags::FORCE,
        )
        .context("create key")?;
    let id = keygen_result.fingerprint().map_err(|err| match err {
        Some(err) => eyre!(err),
        None => eyre!("no fingerprint"),
    })?;

    Ok((id.to_string(), gpg, dir))
}

use async_tempfile::TempDir;
use color_eyre::eyre::{Context as _, Result, eyre};
use gpgme::{Context, CreateKeyFlags, Protocol};

/// Creates a new GPG key for testing in a temporary directory.
///
/// The returned directory is cleaned up when it goes out of scope.
/// The returned string is the fingerprint of the key.
pub async fn emphemeral_gpg_key_id() -> Result<(String, Context, TempDir)> {
    let dir = TempDir::new().await?;
    let mut gpg = Context::from_protocol(Protocol::OpenPgp).context("create gpg context")?;
    gpg.set_engine_home_dir(dir.dir_path().to_string_lossy().as_ref())
        .context("set engine home dir")?;
    gpg.set_armor(true);
    let keygen_result = gpg
        .create_key_with_flags(
            "Attune Test",
            "default",
            Default::default(),
            CreateKeyFlags::NOPASSWD,
        )
        .context("create key")?;
    let id = keygen_result.fingerprint().map_err(|err| match err {
        Some(err) => eyre!(err),
        None => eyre!("no fingerprint"),
    })?;
    Ok((id.to_string(), gpg, dir))
}

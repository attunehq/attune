use sha2::{Digest as _, Sha256};

/// Hashes an API token for storage in the database.
///
/// This function is deterministic for a given secret and token. Use this before
/// storing tokens in the database, and to check whether a token is stored in
/// the database.
pub fn hash_token(secret: &str, token: &str) -> Vec<u8> {
    Sha256::digest(format!("{}{}", secret, token))
        .as_slice()
        .to_vec()
}

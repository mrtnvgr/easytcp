//! Pre-shared authentication tokens.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A pre-shared authentication token.
///
/// A token is created from a secret string with [`Token::new`]. The secret is
/// hashed with SHA-256 and only the digest is stored and exchanged, so the raw
/// secret is never kept in memory.
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Token {
    inner: Vec<u8>,
}

impl Token {
    /// Creates a token by hashing `token` with SHA-256.
    #[must_use]
    pub fn new(token: &str) -> Self {
        let inner = Sha256::digest(token.as_bytes()).to_vec();
        Self { inner }
    }
}

//! Pre-shared authentication tokens.

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// A pre-shared authentication token.
///
/// A token is created from a secret string with [`Token::new`]. The secret is
/// hashed with SHA-256 and only the digest is stored, so the raw secret is never
/// kept in memory.
///
/// During the connection handshake the server sends a random nonce and the
/// client answers with an HMAC of that nonce keyed by the token. Because the
/// nonce is fresh for every connection, a captured response cannot be replayed.
///
/// This authenticates the client to the server but does not encrypt traffic or
/// protect against an active man-in-the-middle; use a secure transport such as
/// TLS when those properties are required.
///
/// Equality is constant-time, so comparing tokens does not leak information
/// through timing.
#[derive(Serialize, Deserialize, Clone)]
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

    /// Computes the HMAC-SHA256 of `nonce` keyed by this token.
    pub(crate) fn compute_mac(&self, nonce: &[u8]) -> Vec<u8> {
        let mut mac =
            HmacSha256::new_from_slice(&self.inner).expect("HMAC accepts keys of any length");
        mac.update(nonce);
        mac.finalize().into_bytes().to_vec()
    }

    /// Verifies `tag` against the HMAC-SHA256 of `nonce`, in constant time.
    pub(crate) fn verify_mac(&self, nonce: &[u8], tag: &[u8]) -> bool {
        self.compute_mac(nonce).as_slice().ct_eq(tag).into()
    }
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        self.inner.as_slice().ct_eq(other.inner.as_slice()).into()
    }
}

impl Eq for Token {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_is_bound_to_the_nonce() {
        let token = Token::new("secret");
        let mac = token.compute_mac(b"nonce-a");

        assert!(token.verify_mac(b"nonce-a", &mac));
        assert!(!token.verify_mac(b"nonce-b", &mac));
    }

    #[test]
    fn mac_requires_the_right_token() {
        let token = Token::new("secret");
        let other = Token::new("other");
        let mac = token.compute_mac(b"nonce");

        assert!(!other.verify_mac(b"nonce", &mac));
    }

    #[test]
    fn equality_is_by_secret() {
        assert!(Token::new("a") == Token::new("a"));
        assert!(Token::new("a") != Token::new("b"));
    }
}

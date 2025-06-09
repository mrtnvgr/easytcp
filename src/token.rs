use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Token {
    inner: Vec<u8>,
}

impl Token {
    pub fn new(token: &str) -> Self {
        let inner = Sha256::digest(token.as_bytes()).to_vec();
        Self { inner }
    }
}

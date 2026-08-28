use sha2::{Digest, Sha256};

pub fn sha256_hex(input: &[u8]) -> String {
    format!("{:x}", Sha256::digest(input))
}

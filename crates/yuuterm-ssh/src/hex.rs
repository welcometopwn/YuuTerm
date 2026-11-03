//! Lowercase hex encoding for digests (sha2 0.11 digests no longer implement `LowerHex`).

use std::fmt::Write;

pub fn lower(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    #[test]
    fn encodes_known_sha256_vector_in_lowercase() {
        assert_eq!(
            super::lower(Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(super::lower([0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
        assert_eq!(super::lower([]), "");
    }
}

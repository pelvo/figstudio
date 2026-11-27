//! SHA-1 digest helper (20-byte digest + 40-char lowercase hex) for Figma `images/<sha1>`
//! archive entries and `ImagePaint.image.hash`, backed by the [`sha1`] and [`hex`] crates.

use sha1::{Digest, Sha1};

/// Computes the 20-byte SHA-1 digest of `data`.
pub fn sha1_bytes(data: &[u8]) -> [u8; 20] {
    Sha1::digest(data).into()
}

/// Computes the 40-character lowercase hexadecimal SHA-1 digest of `data`.
pub fn sha1_hex(data: &[u8]) -> String {
    hex::encode(sha1_bytes(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_matches_standard_vectors_across_all_padding_boundaries() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        // 55 bytes: fits in a single 64-byte block (55 + 1 + 8 = 64).
        assert_eq!(
            sha1_hex(&[b'a'; 55]),
            "c1c8bbdc22796e28c0e15163d20899b65621d65a"
        );
        // 56..=63 bytes: crosses into the two-block padding branch!
        assert_eq!(
            sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        assert_eq!(
            sha1_hex(&[b'a'; 63]),
            "03f09f5b158a7a8cdad920bddc29b81c18a551f5"
        );
        // 64 bytes: exact block boundary.
        assert_eq!(
            sha1_hex(&[b'a'; 64]),
            "0098ba824b5c16427bd7a1122a5a442a25ec644d"
        );
    }
}

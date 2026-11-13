//! Minimal pure-Rust FIPS 180-4 SHA-1 for `.fig` `images/<sha1>` content addressing.

/// Computes the 20-byte SHA-1 digest of `data`.
pub fn sha1_bytes(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x6745_2301;
    let mut h1: u32 = 0xEFCD_AB89;
    let mut h2: u32 = 0x98BA_DCFE;
    let mut h3: u32 = 0x1032_5476;
    let mut h4: u32 = 0xC3D2_E1F0;

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let (chunks, rem) = data.as_chunks::<64>();
    for block in chunks {
        compress_block(block, &mut h0, &mut h1, &mut h2, &mut h3, &mut h4);
    }
    let mut tail = [0u8; 128];
    tail[..rem.len()].copy_from_slice(rem);
    tail[rem.len()] = 0x80;
    let blocks = if rem.len() < 56 { 1 } else { 2 };
    let len_offset = blocks * 64 - 8;
    tail[len_offset..len_offset + 8].copy_from_slice(&bit_len.to_be_bytes());
    for i in 0..blocks {
        let block: &[u8; 64] = tail[i * 64..(i + 1) * 64].try_into().expect("64-byte tail");
        compress_block(block, &mut h0, &mut h1, &mut h2, &mut h3, &mut h4);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

/// Computes the 40-character lowercase hex SHA-1 digest of `data` (matching Figma image keys).
pub fn sha1_hex(data: &[u8]) -> String {
    crate::val::hex(&sha1_bytes(data))
}

fn compress_block(
    block: &[u8; 64],
    h0: &mut u32,
    h1: &mut u32,
    h2: &mut u32,
    h3: &mut u32,
    h4: &mut u32,
) {
    let mut w = [0u32; 80];
    for (i, word) in w.iter_mut().take(16).enumerate() {
        *word = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..80 {
        w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
    }

    let (mut a, mut b, mut c, mut d, mut e) = (*h0, *h1, *h2, *h3, *h4);
    for (i, &wi) in w.iter().enumerate() {
        let (f, k) = match i {
            0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
            20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
            40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
            _ => (b ^ c ^ d, 0xCA62_C1D6),
        };
        let temp = a
            .rotate_left(5)
            .wrapping_add(f)
            .wrapping_add(e)
            .wrapping_add(k)
            .wrapping_add(wi);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = temp;
    }

    *h0 = h0.wrapping_add(a);
    *h1 = h1.wrapping_add(b);
    *h2 = h2.wrapping_add(c);
    *h3 = h3.wrapping_add(d);
    *h4 = h4.wrapping_add(e);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fips_180_4_test_vectors() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
    }
}

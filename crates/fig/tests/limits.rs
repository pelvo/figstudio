//! Crafted inputs that inflate far beyond their size must be refused by the public loader.

use std::io::Write;

use figstudio_fig::Document;

/// A zip whose one member inflates to `size` zero bytes. Zeros compress about 1000:1.
fn zip_with_member(name: &str, size: usize) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        w.start_file(name, opts).unwrap();
        w.write_all(&vec![0u8; size]).unwrap();
        w.finish().unwrap();
    }
    out
}

#[test]
fn a_member_that_inflates_past_the_chunk_limit_is_refused() {
    // One byte over the production per-item limit; the zeros keep the zip itself small.
    let size = figstudio_fig::MAX_CHUNK_BYTES + 1;
    let bytes = zip_with_member("images/bomb", size);
    assert!(
        bytes.len() < 1024 * 1024,
        "the bomb itself stays small: {}",
        bytes.len()
    );
    let err = Document::from_bytes(&bytes)
        .err()
        .expect("bomb must be refused");
    assert!(err.to_string().contains("limit"), "{err}");
}

#[test]
fn a_member_that_is_not_a_fig_is_still_a_normal_error() {
    assert!(Document::from_bytes(&zip_with_member("meta.json", 16)).is_err());
}

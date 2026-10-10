//! Integer fields are checked on write. An out-of-range value is an error, not a wrapped value.

use std::path::PathBuf;

use figstudio_fig::{Document, Node, Val};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(format!("{name}.fig"))
}

/// Puts `value` into `key` of one node of `vector.fig`, then encodes the whole document.
fn encode_with(key: &str, value: i64) -> Result<Vec<u8>, String> {
    let mut doc = Document::open(fixture("vector")).unwrap();
    let mut nodes = doc.nodes.clone();
    let node = &mut nodes[3];
    node.data.set(key, Val::Int(value));
    *node = Node::from_data(node.data.clone()).unwrap();
    doc.nodes = nodes;
    doc.to_bytes().map_err(|e| e.to_string())
}

#[test]
fn a_uint_wider_than_32_bits_is_refused_not_wrapped() {
    // `styleID` is a UINT on Path nodes. 2^32 as `as u32` becomes 0, silently changing the style.
    let key = "styleID";
    let err = encode_with(key, 1i64 << 32).expect_err("2^32 written without error");
    assert!(err.contains("4294967296 does not fit a uint"), "{err}");
}

#[test]
fn a_uint_at_the_32_bit_limit_is_still_written() {
    assert!(encode_with("styleID", i64::from(u32::MAX)).is_ok());
}

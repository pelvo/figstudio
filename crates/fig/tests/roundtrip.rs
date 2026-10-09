use std::io::Read;
use std::path::PathBuf;

use figstudio_fig::{Document, Guid, Node, Val};

const FIXTURES: [&str; 7] = [
    "broken_images",
    "corners",
    "noise_and_grain",
    "sections",
    "stacks_wrap",
    "structure",
    "vector",
];

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(format!("{name}.fig"))
}

/// The decompressed Kiwi message straight from the file, independent of the decoder.
fn raw_message(file: &[u8]) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(file)).unwrap();
    let mut canvas = Vec::new();
    zip.by_name("canvas.fig")
        .unwrap()
        .read_to_end(&mut canvas)
        .unwrap();
    let mut at = 12;
    let mut chunks = Vec::new();
    while at + 4 <= canvas.len() {
        let len = u32::from_le_bytes(canvas[at..at + 4].try_into().unwrap()) as usize;
        chunks.push(canvas[at + 4..at + 4 + len].to_vec());
        at += 4 + len;
    }
    let chunk = &chunks[1];
    let mut out = Vec::new();
    if chunk.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        ruzstd::decoding::StreamingDecoder::new(chunk.as_slice())
            .unwrap()
            .read_to_end(&mut out)
            .unwrap();
    } else {
        flate2::read::DeflateDecoder::new(chunk.as_slice())
            .read_to_end(&mut out)
            .unwrap();
    }
    out
}

fn assert_same(name: &str, a: &Document, b: &Document) {
    assert_eq!(a.nodes.len(), b.nodes.len(), "{name}: node count");
    for (x, y) in a.nodes.iter().zip(&b.nodes) {
        assert_eq!(x.id, y.id, "{name}: node order");
        assert_eq!(x.data, y.data, "{name}: node {} differs", x.id);
    }
    assert_eq!(a.blobs, b.blobs, "{name}: blobs");
    assert_eq!(a.images, b.images, "{name}: images");
    assert_eq!(a.thumbnail, b.thumbnail, "{name}: thumbnail");
    assert_eq!(a.file_name, b.file_name, "{name}: file name");
    assert_eq!(a.pages, b.pages, "{name}: pages");
    assert_eq!(a.format_version, b.format_version, "{name}: version");
}

#[test]
fn every_fixture_survives_write_then_read() {
    for name in FIXTURES {
        let doc = Document::open(fixture(name)).unwrap();
        let bytes = doc.to_bytes().unwrap_or_else(|e| panic!("{name}: {e}"));
        let again = Document::from_bytes(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_same(name, &doc, &again);
        // Writing twice gives identical bytes.
        assert_eq!(
            bytes,
            again.to_bytes().unwrap(),
            "{name}: second write differs"
        );
    }
}

#[test]
fn encoded_message_matches_the_original_bytes() {
    for name in FIXTURES {
        let file = std::fs::read(fixture(name)).unwrap();
        let doc = Document::from_bytes(&file).unwrap();
        let ours = doc.encode_message().unwrap();
        assert!(
            ours == raw_message(&file),
            "{name}: message bytes differ ({} vs {})",
            ours.len(),
            raw_message(&file).len()
        );
    }
}

#[test]
fn edits_are_written() {
    let mut doc = Document::open(fixture("structure")).unwrap();
    let id = Guid(1, 42);
    let mut nodes = doc.nodes.clone();
    let node = nodes.iter_mut().find(|n| n.id == id).unwrap();
    let Val::Obj(map) = &mut node.data else {
        panic!()
    };
    map.insert("name", Val::Str("renamed layer".into()));
    map.insert("visible", Val::Bool(false));
    let edited = Node::from_data(node.data.clone()).unwrap();
    *node = edited;
    doc.set_nodes(nodes);
    let again = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
    let n = again.node(id).unwrap();
    assert_eq!(n.name, "renamed layer");
    assert_eq!(n.data.flag("visible"), Some(false));
}

#[test]
fn unknown_fields_and_wrong_types_are_errors_not_panics() {
    let mut doc = Document::open(fixture("vector")).unwrap();
    let mut nodes = doc.nodes.clone();
    if let Val::Obj(map) = &mut nodes[3].data {
        map.insert("notAFigmaField", Val::Int(1));
    }
    doc.set_nodes(nodes.clone());
    let e = doc.to_bytes().unwrap_err();
    assert!(e.to_string().contains("notAFigmaField"), "{e}");

    if let Val::Obj(map) = &mut nodes[3].data {
        map.remove("notAFigmaField");
        map.insert("opacity", Val::Str("high".into()));
    }
    doc.set_nodes(nodes);
    let e = doc.to_bytes().unwrap_err();
    assert!(e.to_string().contains("expected a number"), "{e}");
}

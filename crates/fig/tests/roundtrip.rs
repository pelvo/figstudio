use std::io::Read;
use std::path::PathBuf;

use figstudio_fig::{Document, Guid, Node, Val};

const FIXTURES: [&str; 10] = [
    "broken_images",
    "corners",
    "motion",
    "motion-probe",
    "noise_and_grain",
    "sections",
    "shaders",
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

/// GUIDs decode inline (no heap object each) and still write back, also when an edit
/// supplies one as the two-field object a JSON round trip produces.
#[test]
fn guids_decode_inline_and_write_back_in_either_form() {
    let mut doc = Document::open(fixture("structure")).unwrap();
    let node = doc.node(Guid(1, 42)).unwrap();
    assert!(matches!(
        node.data.get("guid"),
        Some(Val::Guid(Guid(1, 42)))
    ));
    let parent = node.data.path(&["parentIndex", "guid"]).unwrap().clone();
    assert!(matches!(parent, Val::Guid(_)));
    assert_eq!(parent.as_guid(), node.parent);

    let as_obj: Val = serde_json::from_value(serde_json::to_value(&parent).unwrap()).unwrap();
    assert!(matches!(as_obj, Val::Obj(_)));
    assert_eq!(as_obj, parent);
    let mut nodes = doc.nodes.clone();
    let edited = nodes.iter_mut().find(|n| n.id == Guid(1, 42)).unwrap();
    let Some(Val::Obj(index)) = edited.data.get_mut("parentIndex") else {
        panic!()
    };
    index.insert("guid", as_obj);
    doc.set_nodes(nodes).unwrap();
    let again = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
    assert_eq!(again.node(Guid(1, 42)).unwrap().parent, parent.as_guid());
}

/// A field still encoded when opened (`derivedSymbolData`) is written as edited, not as its
/// original bytes, once changed through `get_mut`.
#[test]
fn an_edited_lazy_field_is_written_as_edited() {
    let mut doc = Document::open(fixture("structure")).unwrap();
    let mut nodes = doc.nodes.clone();
    let node = nodes
        .iter_mut()
        .find(|n| n.data.get("derivedSymbolData").is_some())
        .unwrap();
    let id = node.id;
    let Some(Val::Arr(items)) = node.data.get_mut("derivedSymbolData") else {
        panic!()
    };
    assert!(items.len() > 1);
    items.truncate(1);
    doc.set_nodes(nodes).unwrap();
    let again = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
    let list = again
        .node(id)
        .unwrap()
        .data
        .get("derivedSymbolData")
        .unwrap();
    assert_eq!(list.items().len(), 1);
}

/// An instance's override list, kept encoded inside `symbolData` until read, is written as
/// edited once changed, and other instances' unedited lists keep their bytes.
#[test]
fn an_edited_nested_lazy_field_is_written_as_edited() {
    let mut doc = Document::open(fixture("shaders")).unwrap();
    let overrides = |data: &Val| {
        data.path(&["symbolData", "symbolOverrides"])
            .map(|v| v.items().len())
    };
    let mut nodes = doc.nodes.clone();
    let mut with = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| overrides(&n.data).is_some_and(|len| len > 1))
        .map(|(i, n)| (i, n.id, overrides(&n.data)));
    let (i, edited, _) = with.next().unwrap();
    let (_, other, other_len) = with.next().unwrap();
    let Some(Val::Arr(list)) = nodes[i]
        .data
        .get_mut("symbolData")
        .and_then(|s| s.get_mut("symbolOverrides"))
    else {
        panic!()
    };
    list.truncate(1);
    doc.set_nodes(nodes).unwrap();
    let again = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
    assert_eq!(overrides(&again.node(edited).unwrap().data), Some(1));
    assert_eq!(overrides(&again.node(other).unwrap().data), other_len);
}

/// Two copies of a document compare equal without decoding their still-encoded fields
/// (as compare does for every unchanged node), and a real difference is still seen.
#[test]
fn equal_documents_compare_equal_without_decoding_their_lazy_fields() {
    let encoded = |doc: &Document| -> usize {
        doc.nodes
            .iter()
            .map(|n| match &n.data {
                Val::Obj(o) => o.len() - o.values_decoded().count(),
                _ => 0,
            })
            .sum()
    };
    let a = Document::open(fixture("structure")).unwrap();
    let mut b = Document::open(fixture("structure")).unwrap();
    let before = encoded(&a);
    assert!(before > 0);
    assert!(a.nodes.iter().zip(&b.nodes).all(|(x, y)| x.data == y.data));
    assert_eq!((encoded(&a), encoded(&b)), (before, before));

    let mut nodes = b.nodes.clone();
    let node = nodes
        .iter_mut()
        .find(|n| n.data.get("derivedSymbolData").is_some())
        .unwrap();
    let Some(Val::Arr(items)) = node.data.get_mut("derivedSymbolData") else {
        panic!()
    };
    items.truncate(1);
    b.set_nodes(nodes).unwrap();
    assert!(!a.nodes.iter().zip(&b.nodes).all(|(x, y)| x.data == y.data));
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
    doc.set_nodes(nodes).unwrap();
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
    doc.set_nodes(nodes.clone()).unwrap();
    let e = doc.to_bytes().unwrap_err();
    assert!(e.to_string().contains("notAFigmaField"), "{e}");

    if let Val::Obj(map) = &mut nodes[3].data {
        map.remove("notAFigmaField");
        map.insert("opacity", Val::Str("high".into()));
    }
    doc.set_nodes(nodes).unwrap();
    let e = doc.to_bytes().unwrap_err();
    assert!(e.to_string().contains("expected a number"), "{e}");
}

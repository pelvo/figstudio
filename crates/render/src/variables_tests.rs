use figstudio_fig::{Document, Val};

use super::Variables;

fn still_encoded(doc: &Document) -> usize {
    doc.nodes
        .iter()
        .map(|n| match &n.data {
            Val::Obj(o) => o.len() - o.values_decoded().count(),
            _ => 0,
        })
        .sum()
}

/// A style reference in a decoded field is found without decoding the heavy fields kept
/// encoded (`derivedSymbolData`); a document mentioning none is still searched in full.
#[test]
fn a_style_reference_is_found_without_decoding_the_heavy_fields() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/structure.fig"
    );
    let doc = Document::open(path).unwrap();
    let encoded = still_encoded(&doc);
    assert!(encoded > 0);
    assert!(!Variables::new(&doc).used);
    assert_eq!(
        still_encoded(&doc),
        0,
        "with no reference, everything is searched"
    );

    let mut doc = Document::open(path).unwrap();
    let mut nodes = doc.nodes.clone();
    let last = nodes.last_mut().unwrap();
    last.data
        .set("styleIdForFill", Val::Obj(figstudio_fig::Obj::new()));
    doc.set_nodes(nodes).unwrap();
    let encoded = still_encoded(&doc);
    assert!(encoded > 0);
    assert!(Variables::new(&doc).used);
    assert_eq!(still_encoded(&doc), encoded);
}

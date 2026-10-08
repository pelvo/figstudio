use std::path::PathBuf;

use figstudio_fig::{Document, Guid};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").join(format!("{name}.fig"))
}

/// Node counts and format versions, cross-checked against the TypeScript decoder in figma-read-mcp.
const FIXTURES: [(&str, u32, usize); 7] = [
    ("broken_images", 20, 6),
    ("corners", 106, 13),
    ("noise_and_grain", 87, 6),
    ("sections", 106, 21),
    ("stacks_wrap", 106, 57),
    ("structure", 48, 26),
    ("vector", 20, 5),
];

#[test]
fn decodes_every_fixture_with_its_own_schema() {
    for (name, version, nodes) in FIXTURES {
        let doc = Document::open(fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(doc.prelude, "fig-kiwi", "{name}");
        assert_eq!(doc.format_version, version, "{name}");
        assert_eq!(doc.nodes.len(), nodes, "{name}");
        assert!(doc.root.is_some(), "{name}");
        assert_eq!(doc.pages.len(), 1, "{name}");
        assert_eq!(doc.internal_pages.len(), 1, "{name}");
        assert!(doc.thumbnail.is_some(), "{name}");
    }
}

#[test]
fn builds_the_tree_in_sibling_order() {
    let doc = Document::open(fixture("structure")).unwrap();
    let page = doc.pages[0];
    assert_eq!(doc.node(page).unwrap().name, "Page 1");
    let top: Vec<&str> = doc.children(page).iter().map(|id| doc.node(*id).unwrap().name.as_str()).collect();
    assert_eq!(top, ["Groups", "Symbols and images"]);
    let text = doc.node(Guid(1, 42)).unwrap();
    assert_eq!(text.kind, "TEXT");
    assert_eq!(text.parent, Some(Guid(1, 37)));
    assert_eq!(text.data.path(&["textData", "characters"]).and_then(|v| v.as_str()), Some("ABC"));
    assert_eq!(doc.page_of(Guid(1, 42)), Some(page));
    assert_eq!(doc.descendant_count(Guid(1, 53)), 4);
}

#[test]
fn reads_metadata_images_and_blobs() {
    let doc = Document::open(fixture("structure")).unwrap();
    assert_eq!(doc.file_name.as_deref(), Some("structure"));
    assert_eq!(doc.images.len(), 3);
    assert!(doc.images.keys().all(|k| k.len() == 40 && k.chars().all(|c| c.is_ascii_hexdigit())));
    assert_eq!(doc.blobs.len(), 28);
}

#[test]
fn refuses_files_that_are_not_fig() {
    assert!(Document::from_bytes(b"not a figma file at all").is_err());
    assert!(Document::from_bytes(b"").is_err());
    assert!(Document::from_bytes(b"PK\x03\x04 broken zip").is_err());
    let mut fake = b"fig-kiwi".to_vec();
    fake.extend_from_slice(&20u32.to_le_bytes());
    fake.extend_from_slice(&100u32.to_le_bytes());
    assert!(Document::from_bytes(&fake).is_err(), "truncated chunk must fail");
}

#[test]
fn guid_parses_and_prints() {
    let id: Guid = "12:34".parse().unwrap();
    assert_eq!(id, Guid(12, 34));
    assert_eq!(id.to_string(), "12:34");
    assert!("1234".parse::<Guid>().is_err());
}

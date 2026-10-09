//! Data that is present in a file must survive a save, even when it is empty.

use std::path::PathBuf;

use figstudio_fig::Document;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(format!("{name}.fig"))
}

#[test]
fn a_present_but_empty_image_survives_a_save() {
    let mut doc = Document::open(fixture("structure")).unwrap();
    let before = doc.images.len();
    doc.images.insert("00".repeat(20), Vec::new());
    let again = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
    assert_eq!(
        again.images.len(),
        before + 1,
        "empty image was dropped on save"
    );
    assert_eq!(again.images.get(&"00".repeat(20)).map(Vec::len), Some(0));
}

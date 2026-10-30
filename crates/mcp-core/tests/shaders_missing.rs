//! Which shaders of a file have no source, and sources read from zip bytes (a dropped or
//! picked "View code" download).

use std::io::Write;

use figstudio_fig::Document;
use figstudio_mcp_core::shaders::{self, ShaderSource};

fn doc() -> Document {
    Document::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/shaders.fig"
    ))
    .unwrap()
}

fn concentric_zip() -> Vec<u8> {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/shader-sources/concentric"
    );
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for f in ["features.json", "main.ts"] {
        z.start_file(f, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(&std::fs::read(format!("{dir}/{f}")).unwrap())
            .unwrap();
    }
    z.finish().unwrap().into_inner()
}

#[test]
fn every_used_shader_is_missing_until_its_source_is_added() {
    let doc = doc();
    let missing = shaders::missing(&doc, &[]);
    assert_eq!(missing.len(), 12, "{missing:?}");
    let concentric = missing
        .iter()
        .find(|m| m.layers.contains(&"1:14622".parse().unwrap()))
        .expect("the concentric layer");
    assert_eq!(concentric.kind, "fill");
    assert_eq!(concentric.id, "1650660541440366682");

    // A zip of its "View code" download, labelled with the shader's ID.
    let source = ShaderSource::from_zip(concentric.id.clone(), &concentric_zip()).unwrap();
    assert_eq!(source.name.as_deref(), Some("Concentric patterns"));
    assert!(source.has_code());
    let after = shaders::missing(&doc, &[source]);
    assert_eq!(after.len(), 11);
    assert!(after.iter().all(|m| m.id != concentric.id));
}

#[test]
fn a_bad_zip_is_no_source() {
    assert!(ShaderSource::from_zip("x".into(), b"not a zip").is_none());
    assert!(ShaderSource::from_zip("x".into(), &[]).is_none());
}

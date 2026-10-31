//! A file using shaders without their code: the notice, the markers and the zips that fill
//! them in.
use std::io::Write;

use super::*;

/// The Concentric patterns layer of shaders.fig, and its shader's ID.
const CONCENTRIC: Guid = Guid(1, 14622);
const CONCENTRIC_ID: &str = "1650660541440366682";

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
fn a_file_with_shaders_but_no_code_says_so_and_takes_their_zips() {
    let mut h = harness_fixture("shaders.fig", 1.0 / 60.0);
    let mut c = Vec::new();
    step(&mut h, &mut c);
    let missing = |h: &Harness<'_, App>| h.state().tabs[0].shaders.missing.len();
    assert_eq!(missing(&h), 12);
    // The notice offers the zips and the steps to get them.
    click_label(&mut h, &mut c, "How to get them");
    assert!(h.query_by_label_contains("View code").is_some());
    click_label(&mut h, &mut c, "Choose shader zips…");
    assert_eq!(h.state().tabs[0].shaders.choose.take(), Some(None));

    // The zip of one shader's code: that shader is drawn, the notice counts one less.
    let id = h.state().tabs[0].id;
    h.state()
        .shader_code
        .tx
        .send(crate::shader_code::Zip {
            tab: id,
            name: "Concentric-patterns-source.zip".into(),
            shader: Some(CONCENTRIC_ID.into()),
            bytes: concentric_zip(),
        })
        .unwrap();
    step(&mut h, &mut c);
    assert_eq!(missing(&h), 11);
    assert!(
        h.state().tabs[0]
            .shaders
            .missing
            .iter()
            .all(|m| m.id != CONCENTRIC_ID)
    );
    assert!(
        h.state()
            .toasts
            .iter()
            .any(|t| t.text == "Added the code of Concentric patterns" && !t.error)
    );

    // Not a zip: an error, nothing changes.
    h.state()
        .shader_code
        .tx
        .send(crate::shader_code::Zip {
            tab: id,
            name: "notes.zip".into(),
            shader: None,
            bytes: b"plain text".to_vec(),
        })
        .unwrap();
    step(&mut h, &mut c);
    assert_eq!(missing(&h), 11);
    assert!(
        h.state()
            .toasts
            .iter()
            .any(|t| t.error && t.text.contains("not a zip"))
    );

    // Closing the notice keeps it closed.
    h.get_by_label("Hide").click();
    for _ in 0..3 {
        step(&mut h, &mut c);
    }
    assert!(h.state().tabs[0].shaders.notice_closed);
    assert!(h.query_by_label("Choose shader zips…").is_none());
    // The warning chip (shown once tiles report what they skipped) brings it back.
    for _ in 0..400 {
        if h.query_by_label("Page warnings").is_some() {
            break;
        }
        step(&mut h, &mut c);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.get_by_label("Page warnings").click();
    for _ in 0..3 {
        step(&mut h, &mut c);
    }
    assert!(!h.state().tabs[0].shaders.notice_closed);
    assert!(h.query_by_label("Choose shader zips…").is_some());
    let _ = CONCENTRIC;
}

#[test]
fn a_shader_marker_asks_for_that_shaders_code() {
    let mut h = harness_fixture("shaders.fig", 1.0 / 60.0);
    let mut c = Vec::new();
    h.state_mut().tabs[0].fit_node = Some(CONCENTRIC);
    for _ in 0..4 {
        step(&mut h, &mut c);
    }
    h.get_by_label_contains("Shader needs its code").click();
    for _ in 0..2 {
        step(&mut h, &mut c);
    }
    assert_eq!(
        h.state().tabs[0].shaders.choose.take(),
        Some(Some(CONCENTRIC_ID.to_owned()))
    );
}

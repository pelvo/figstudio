//! What the app registers for a document (its decoded images) is freed when the document's
//! last tab closes, however many edits the tab went through.
use super::*;
use crate::editor::apply_tab_ops;
use figstudio_fig::edit::EditOp;
use figstudio_render::images::cache_key;

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn closing_a_tab_after_an_edit_frees_its_cached_images() {
    let mut h = harness();
    let app = h.state_mut();
    let images = app.workers.images();
    let tab = &mut app.tabs[0];
    let opened = tab.doc.instance;
    apply_tab_ops(
        tab,
        &[EditOp::Rename {
            target: Guid(1, 25),
            name: "renamed".into(),
        }],
        Some("Rename layer"),
        None,
    )
    .unwrap();
    assert_ne!(
        tab.doc.instance, opened,
        "precondition: the edit made a new revision"
    );
    assert!(images.insert_encoded(&cache_key(&tab.doc, "ab"), &png(), 0));
    assert!(images.bytes() > 0);

    app.close_now(0);

    assert_eq!(images.bytes(), 0, "the closed tab's images stayed cached");
}

//! Export crop (`render_export_complete` / `render_export_svg_complete`): like Figma's
//! export, the drawn pixels, never less than a frame-like layer's box.
use figstudio_fig::{Document, Guid};
use figstudio_render::{Renderer, node_bounds};

fn open(name: &str) -> Document {
    Document::open(format!(
        "{}/../../tests/fixtures/{name}.fig",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn png_size(doc: &Document, id: Guid) -> (u16, u16) {
    let f = Renderer::new()
        .render_export_complete(doc, id, 1.0, None, |_| None)
        .unwrap();
    (f.pixmap.width(), f.pixmap.height())
}

fn svg_size(doc: &Document, id: Guid) -> String {
    let svg = Renderer::new()
        .render_export_svg_complete(doc, id, 1.0, None, usize::MAX, |_| None)
        .unwrap();
    let m = &svg.markup;
    let w = m
        .split("width=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let h = m
        .split("height=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    format!("{w}x{h}")
}

#[test]
fn text_is_cropped_to_its_glyphs_not_its_layout_box() {
    let doc = open("shaders");
    // A 1580 x 40 centred label: Figma exports it as 330 x 33.
    let label = Guid(1, 14623);
    let (w, h) = png_size(&doc, label);
    assert!((328..=333).contains(&w), "{w}");
    assert!((31..=35).contains(&h), "{h}");
    assert_eq!(svg_size(&doc, label), format!("{w}x{h}"));
}

#[test]
fn a_frame_keeps_its_own_box() {
    let doc = open("shaders");
    // 1580 x 935, clipping; its shader fill is not drawn, but the frame's box stays.
    let frame = Guid(1, 14622);
    assert_eq!(png_size(&doc, frame), (1580, 935));
    assert_eq!(svg_size(&doc, frame), "1580x935");
}

#[test]
fn a_shadow_widens_the_crop_past_the_box() {
    let doc = open("structure");
    // A group-like frame with a 4 px drop shadow, offset 4 down.
    let layer = Guid(1, 53);
    let b = node_bounds(&doc, layer).unwrap();
    let (w, h) = png_size(&doc, layer);
    assert!(
        f64::from(w) > b.width() && f64::from(h) > b.height(),
        "{w}x{h} vs {b:?}"
    );
    assert_eq!(svg_size(&doc, layer), format!("{w}x{h}"));
}

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
    assert_eq!(png_size(&doc, label), (330, 33));
    assert_eq!(svg_size(&doc, label), "330x33");
}

/// The `viewBox` of an export: `x y w h`.
fn svg_view(doc: &Document, id: Guid) -> [f64; 4] {
    let svg = Renderer::new()
        .render_export_svg_complete(doc, id, 1.0, None, usize::MAX, |_| None)
        .unwrap();
    let view = svg.markup.split("viewBox=\"").nth(1).unwrap();
    let nums: Vec<f64> = view
        .split('"')
        .next()
        .unwrap()
        .split(' ')
        .map(|n| n.parse().unwrap())
        .collect();
    [nums[0], nums[1], nums[2], nums[3]]
}

#[test]
fn the_crop_starts_where_the_drawing_does_and_rounds_only_its_size() {
    let doc = open("corners");
    // Stroked shapes whose ink starts between pixels. Figma's Copy as SVG gives 75 x 75 and
    // 81 x 74 with the drawing flush against the top-left corner.
    for (id, size) in [(Guid(2051, 15), (75, 75)), (Guid(2051, 16), (81, 74))] {
        let [x, y, w, h] = svg_view(&doc, id);
        assert!(x.fract().abs() > 1e-3 || y.fract().abs() > 1e-3, "{x} {y}");
        assert_eq!((w, h), (f64::from(size.0), f64::from(size.1)));
        assert_eq!(png_size(&doc, id), (size.0, size.1));
    }
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

/// The most-covered pixel of the first row and of the first column of a PNG export (alpha).
fn edge_alpha(doc: &Document, id: Guid) -> (u8, u8) {
    let f = Renderer::new()
        .render_export_complete(doc, id, 1.0, None, |_| None)
        .unwrap();
    let w = usize::from(f.pixmap.width());
    let px = f.pixmap.data_as_u8_slice();
    let alpha = |x: usize, y: usize| px[(y * w + x) * 4 + 3];
    let h = usize::from(f.pixmap.height());
    let row = (0..w).map(|x| alpha(x, 0)).max().unwrap();
    let col = (0..h).map(|y| alpha(0, y)).max().unwrap();
    (row, col)
}

#[test]
fn a_png_export_is_drawn_from_the_crop_corner_like_the_svg() {
    let doc = open("corners");
    // The stroke touches the crop's top and left sides, so the first row and column are
    // nearly covered. Drawn from a whole pixel instead, they were at most 187 and 130.
    for id in [Guid(2051, 15), Guid(2051, 16)] {
        let (row, col) = edge_alpha(&doc, id);
        assert!(row >= 215 && col >= 215, "{id:?}: {row} {col}");
    }
}

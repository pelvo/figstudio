//! `download_assets` with `format: svg`: valid markup that draws what the PNG render draws.
use figstudio_fig::Document;
use figstudio_mcp_core::{
    assets::{AssetFormat, download},
    output::{Block, Output},
    rendering::Rendering,
    screenshot::{render_node, svg_node},
};
use std::path::PathBuf;

fn open(name: &str) -> Document {
    Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../tests/fixtures/{name}.fig")),
    )
    .unwrap()
}

fn texts(out: &Output) -> Vec<&str> {
    out.content.iter().filter_map(Block::as_text).collect()
}

/// Root-mean-square difference (0 to 1) between the PNG render of `id` and its SVG export
/// rasterized by resvg, both premultiplied RGBA at scale 1.
fn difference(doc: &Document, id: &str) -> f64 {
    let rendering = Rendering::default();
    let png = render_node(doc, id, Some(1.0), &rendering)
        .unwrap()
        .frame
        .pixmap;
    let svg = svg_node(doc, id, Some(1.0), &rendering, usize::MAX).unwrap();
    let tree = resvg::usvg::Tree::from_str(&svg.svg.markup, &resvg::usvg::Options::default())
        .expect("the export parses as SVG");
    let (w, h) = (u32::from(png.width()), u32::from(png.height()));
    assert_eq!((u32::from(svg.width), u32::from(svg.height)), (w, h));
    let mut raster = resvg::tiny_skia::Pixmap::new(w, h).unwrap();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut raster.as_mut(),
    );
    let a = png.data_as_u8_slice();
    let b = raster.data();
    assert_eq!(a.len(), b.len());
    let sum: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2))
        .sum();
    (sum / a.len() as f64).sqrt() / 255.0
}

#[test]
fn svg_comes_back_as_a_line_then_the_markup() {
    let doc = open("structure");
    let out = download(
        &doc,
        &["1:34".to_owned()],
        AssetFormat::Svg,
        Some(1.0),
        false,
        &Rendering::default(),
    );
    assert!(!out.is_error, "{:?}", texts(&out));
    let texts = texts(&out);
    assert_eq!(texts.len(), 2, "{texts:?}");
    assert!(texts[0].starts_with("1:34: "), "{}", texts[0]);
    assert!(texts[0].contains("bytes of SVG"), "{}", texts[0]);
    assert!(texts[1].starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
    assert!(texts[1].ends_with("</svg>"));
}

#[test]
fn svg_draws_what_the_png_render_draws() {
    // Vector paths and strokes, text as outlines, and frames with clips.
    for (file, id) in [("vector", "0:3"), ("structure", "1:34"), ("corners", "0:3")] {
        let doc = open(file);
        let id = doc
            .children(doc.pages[0])
            .first()
            .map_or(id.to_owned(), |g| format!("{}:{}", g.0, g.1));
        let diff = difference(&doc, &id);
        assert!(diff < 0.03, "{file} {id}: {diff}");
    }
}

#[test]
fn outside_strokes_are_erased_inside_the_shape() {
    // The section frames draw outside strokes: a wide stroke with the fill shape erased
    // (destination-out), which the export expresses as a mask. Without it the inner half of
    // the dashed band covers the content and the difference is about 0.06.
    let doc = open("sections");
    let first = doc.children(doc.pages[0])[0];
    let diff = difference(&doc, &format!("{}:{}", first.0, first.1));
    assert!(diff < 0.02, "{diff}");
}

#[test]
fn an_export_over_the_limit_is_an_error() {
    let doc = open("structure");
    let error = svg_node(&doc, "1:34", Some(1.0), &Rendering::default(), 100)
        .err()
        .expect("100 bytes is too small");
    assert!(error.to_string().contains("over 100 bytes"), "{error}");
}

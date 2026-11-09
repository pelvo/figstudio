//! SVG export embeds an image paint's original file, as Figma's Copy as SVG does, instead of
//! re-encoding pixels decoded at the size of the view.
use figstudio_fig::{Document, Guid};
use figstudio_render::Renderer;

fn path(name: &str) -> String {
    format!(
        "{}/../../tests/fixtures/{name}.fig",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// The `<image>` elements of `markup`: (width, height, media type).
fn images(markup: &str) -> Vec<(String, String, String)> {
    let attr = |tag: &str, name: &str| {
        tag.split(&format!(" {name}=\""))
            .nth(1)
            .and_then(|v| v.split('"').next())
            .unwrap_or_default()
            .to_owned()
    };
    markup
        .split("<image ")
        .skip(1)
        .map(|tag| {
            let mime = tag
                .split("href=\"data:")
                .nth(1)
                .and_then(|v| v.split(';').next())
                .unwrap_or_default()
                .to_owned();
            (attr(tag, "width"), attr(tag, "height"), mime)
        })
        .collect()
}

fn expected() -> Vec<(String, String, String)> {
    // The 1024 x 512 JPEG and the 64 x 64 PNG in the "Symbols and images" frame, drawn far
    // smaller than their size. Figma embeds both files unchanged.
    let mut v = vec![
        ("1024".into(), "512".into(), "image/jpeg".into()),
        ("64".into(), "64".into(), "image/png".into()),
    ];
    v.sort();
    v
}

#[test]
fn an_svg_embeds_the_original_image_files() {
    let doc = Document::open(path("structure")).unwrap();
    let svg = Renderer::new()
        .render_export_svg_complete(&doc, Guid(1, 4), 1.0, None, usize::MAX, |_| None)
        .unwrap();
    let mut found = images(&svg.markup);
    found.sort();
    assert_eq!(found, expected());
}

#[test]
fn a_lazy_document_reads_the_original_files_for_its_svg() {
    let doc = Document::open_lazy(path("structure")).unwrap();
    assert!(doc.images.is_empty());
    let mut file = std::fs::File::open(path("structure")).unwrap();
    let svg = Renderer::new()
        .render_export_svg_complete(&doc, Guid(1, 4), 1.0, None, usize::MAX, |want| {
            figstudio_fig::archive::read_entry(&mut file, &want.entry).ok()
        })
        .unwrap();
    let mut found = images(&svg.markup);
    found.sort();
    assert_eq!(found, expected());
}

#[test]
fn a_lazy_document_exports_the_same_pixels_as_a_whole_one() {
    let whole = Document::open(path("structure")).unwrap();
    let lazy = Document::open_lazy(path("structure")).unwrap();
    let mut file = std::fs::File::open(path("structure")).unwrap();
    let a = Renderer::new()
        .render_export_complete(&whole, Guid(1, 4), 1.0, None, |_| None)
        .unwrap();
    let b = Renderer::new()
        .render_export_complete(&lazy, Guid(1, 4), 1.0, None, |want| {
            figstudio_fig::archive::read_entry(&mut file, &want.entry).ok()
        })
        .unwrap();
    assert_eq!(b.stats.images_loading, 0);
    assert!(a.pixmap.data() == b.pixmap.data());
}

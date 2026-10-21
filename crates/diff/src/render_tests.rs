//! Memory knobs of the comparison renderers: image cache budget and the mask-only compare.
use crate::RenderOpts;
use crate::render::side_renderer;
use crate::render_pixels::{compare, compare_mask};
use figstudio_fig::{Document, Node, Obj, Val};
use figstudio_render::{Frame, Viewport, kurbo::Affine};
use std::path::PathBuf;

fn structure() -> Document {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    Document::open(&path).unwrap()
}

fn png(side: u32, tag: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(side, side, |x, y| {
        image::Rgba([(x ^ tag) as u8, (y ^ (tag >> 8)) as u8, tag as u8, 255])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// Gives the page's first child an IMAGE fill of `hash` (lowercase hex SHA-1).
fn with_image_fill(doc: &mut Document, hash: &str) {
    let target = doc.children(doc.pages[0])[0];
    let mut nodes = doc.nodes.clone();
    let node = nodes.iter_mut().find(|n| n.id == target).unwrap();
    let bytes: Vec<u8> = (0..hash.len() / 2)
        .map(|i| u8::from_str_radix(&hash[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    let mut image = Obj::new();
    image.insert("hash", Val::Bytes(bytes));
    let mut paint = Obj::new();
    paint.insert("type", Val::Enum("IMAGE"));
    paint.insert("visible", Val::Bool(true));
    paint.insert("opacity", Val::Float(1.0));
    paint.insert("image", Val::Obj(image));
    paint.insert("imageScaleMode", Val::Enum("STRETCH"));
    node.data.set("fillPaints", Val::Arr(vec![Val::Obj(paint)]));
    *node = Node::from_data(node.data.clone()).unwrap();
    doc.set_nodes(nodes).unwrap();
}

#[test]
fn the_image_cache_budget_option_reaches_both_comparison_renderers() {
    let budget = 3 * 1024 * 1024; // holds two 512x512 images, not eight
    let opts = RenderOpts {
        image_cache_bytes: budget,
        system_fonts: false,
        ..RenderOpts::default()
    };
    let mut renderer = side_renderer(&opts);
    let mut peak = 0;
    for tag in 0..8u32 {
        let mut doc = structure();
        let hash = format!("{tag:040x}");
        doc.images.insert(hash.clone(), png(512, tag));
        with_image_fill(&mut doc, &hash);
        renderer
            .render_fit(&doc, doc.pages[0], 1.0, 0.0, None)
            .unwrap();
        peak = peak.max(renderer.image_cache_bytes());
    }
    assert!(peak > 0 && peak <= budget, "peak {peak}, budget {budget}");
    // The default is far below the viewer's 1 GiB.
    assert!(RenderOpts::default().image_cache_bytes < 1 << 30);
}

#[test]
fn mask_only_compare_matches_the_mask_of_the_full_compare() {
    let doc = structure();
    let draw = |dx: f64| {
        let mut renderer = side_renderer(&RenderOpts {
            system_fonts: false,
            ..RenderOpts::default()
        });
        renderer.render_complete(
            &doc,
            doc.pages[0],
            &Viewport {
                width: 200,
                height: 150,
                transform: Affine::translate((dx, 0.0)),
                background: None,
            },
            |_| None,
        )
    };
    let (a, b): (Option<Frame>, Option<Frame>) = (Some(draw(0.0)), Some(draw(7.0)));
    let bg = [[0; 4], [255; 4]];
    for (before, after) in [(&a, &b), (&a, &None), (&None, &b), (&None, &None)] {
        for (old_bg, new_bg) in [(bg[0], bg[0]), (bg[0], bg[1])] {
            let (changed, _, _, full) = compare(before, after, old_bg, new_bg, 200, 150, 8);
            let mask = compare_mask(before, after, (old_bg, new_bg), (200, 150), 8);
            assert_eq!(mask, full);
            assert_eq!(mask.iter().filter(|&&c| c).count() as u64, changed);
        }
    }
    let (changed, ..) = compare(&a, &b, [0; 4], [0; 4], 200, 150, 8);
    assert!(changed > 0, "the shifted render must differ");
}

//! The image cache keeps decoded pixels across renders. It must stay under its byte bound,
//! never keep failed decodes, and never mix up images of different documents.

use std::io::Cursor;
use std::path::PathBuf;

use figstudio_fig::{Document, Node, Obj, Val};
use figstudio_render::Renderer;
use figstudio_render::images::Images;

/// A valid PNG of `side` x `side` pixels whose content depends on `tag`.
fn png(side: u32, tag: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(side, side, |x, y| {
        image::Rgba([(x ^ tag) as u8, (y ^ (tag >> 8)) as u8, tag as u8, 255])
    });
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// Gives the first node of the page an IMAGE fill that refers to `hash` (lowercase hex SHA-1).
fn with_image_fill(doc: &mut Document, hash: &str) {
    let page = doc.pages[0];
    let target = doc.children(page)[0];
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

fn structure() -> Document {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    Document::open(&path).unwrap()
}

#[test]
fn the_cache_stays_under_its_byte_bound_across_many_documents() {
    // 80 distinct images, each drawn in its own document at full size: far more than the
    // budget, so the bound is only met if older entries are evicted.
    let budget = 8 * 1024 * 1024;
    let mut r = Renderer::with_images(Images::new(budget));
    r.system_fonts = false;
    let mut peak = 0usize;
    for tag in 0..80u32 {
        let mut doc = structure();
        let hash = format!("{tag:040x}");
        doc.images.insert(hash.clone(), png(512, tag));
        with_image_fill(&mut doc, &hash);
        let page = doc.pages[0];
        let frame = r.render_fit(&doc, page, 1.0, 0.0, None).unwrap();
        assert_eq!(frame.stats.missing_images, 0);
        peak = peak.max(r.image_cache_bytes());
    }
    assert!(
        peak <= budget,
        "cache grew to {peak} bytes, budget {budget}"
    );
    assert!(peak > 0, "nothing was cached, the test saw no images");
}

#[test]
fn failed_decodes_are_not_retained() {
    let images = Images::new(1024 * 1024);
    for i in 0..500 {
        assert!(!images.insert_encoded(&format!("junk{i}"), &[0, 1, 2], 0));
    }
    assert_eq!(images.bytes(), 0, "failed decodes hold no pixels");
    assert_eq!(images.in_flight(), 0);
    // And a document whose only image does not decode draws it as missing, every time.
    let mut doc = structure();
    let hash = format!("{:040x}", 3u32);
    doc.images.insert(hash.clone(), vec![0, 1, 2]);
    with_image_fill(&mut doc, &hash);
    let mut r = Renderer::with_images(images.clone());
    r.system_fonts = false;
    // (The fixture's own images decode and are cached; only the broken one is missing.)
    let frame = r.render_fit(&doc, doc.pages[0], 1.0, 0.0, None).unwrap();
    assert_eq!(frame.stats.missing_images, 1);
    let after_first = images.bytes();
    let frame = r.render_fit(&doc, doc.pages[0], 1.0, 0.0, None).unwrap();
    assert_eq!(frame.stats.missing_images, 1);
    assert_eq!(
        images.bytes(),
        after_first,
        "the broken image was not kept or retried"
    );
}

#[test]
fn a_document_without_the_bytes_does_not_poison_the_cache_for_one_that_has_them() {
    let hash = format!("{:040x}", 7u32);
    let mut with = structure();
    with.images.insert(hash.clone(), png(16, 7));
    with_image_fill(&mut with, &hash);
    let mut without = structure();
    with_image_fill(&mut without, &hash);
    let mut conflict = structure();
    conflict.images.insert(hash.clone(), png(16, 99));
    with_image_fill(&mut conflict, &hash);

    let mut r = Renderer::new();
    r.system_fonts = false;
    let render =
        |r: &mut Renderer, doc: &Document| r.render_fit(doc, doc.pages[0], 1.0, 0.0, None).unwrap();
    let none = render(&mut r, &without);
    assert_eq!(none.stats.missing_images, 1, "no bytes, so no image");
    let one = render(&mut r, &with);
    assert_eq!(
        one.stats.missing_images, 0,
        "a missing lookup must not cache a miss for the document that has the bytes"
    );
    let none = render(&mut r, &without);
    assert_eq!(
        none.stats.missing_images, 1,
        "a document without bytes never inherits a cached image"
    );
    let two = render(&mut r, &conflict);
    assert_eq!(two.stats.missing_images, 0);
    assert!(
        one.pixmap.data_as_u8_slice() != two.pixmap.data_as_u8_slice(),
        "same hash with different bytes must not alias"
    );
    // A fresh renderer draws the conflicting document the same way: nothing leaked.
    let mut fresh = Renderer::new();
    fresh.system_fonts = false;
    assert!(
        render(&mut fresh, &conflict).pixmap.data_as_u8_slice() == two.pixmap.data_as_u8_slice()
    );
}

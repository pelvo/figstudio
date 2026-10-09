//! The image cache keeps decoded pixels across renders. It must stay under its byte bound.

use std::io::Cursor;
use std::path::PathBuf;

use figstudio_fig::{Document, Node, Obj, Val};
use figstudio_render::kurbo::Affine;
use figstudio_render::paint::{IMAGE_CACHE_MAX_ENTRIES, ImageCache};
use figstudio_render::{Renderer, Viewport};

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

#[test]
fn the_cache_stays_under_its_byte_bound_across_many_documents() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let mut r = Renderer::new();
    r.system_fonts = false;
    let mut peak = 0usize;
    // 80 distinct 4 MiB images are 320 MiB decoded, more than the 256 MiB cache limit, so the
    // bound is only met if older entries are evicted.
    for tag in 0..80u32 {
        let mut doc = Document::open(&path).unwrap();
        // Each document carries its own distinct image: a 1024 x 1024 RGBA decodes to 4 MiB.
        let hash = format!("{tag:040x}");
        doc.images.insert(hash.clone(), png(1024, tag));
        with_image_fill(&mut doc, &hash);
        let page = doc.pages[0];
        r.render(
            &doc,
            page,
            &Viewport {
                width: 64,
                height: 64,
                transform: Affine::IDENTITY,
                background: None,
            },
        );
        peak = peak.max(r.image_cache_bytes());
    }
    let limit = figstudio_render::paint::IMAGE_CACHE_MAX_BYTES;
    assert!(peak <= limit, "cache grew to {peak} bytes, limit {limit}");
    assert!(peak > 0, "nothing was cached, the test saw no images");
}

#[test]
fn failed_decodes_are_not_retained_and_evicted_arc_remains_valid() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let mut doc = Document::open(&path).unwrap();
    let mut cache = ImageCache::default();
    for i in 0..(IMAGE_CACHE_MAX_ENTRIES * 2) {
        let hash = format!("{i:040x}");
        doc.images.insert(hash.clone(), vec![0, 1, 2]);
        assert!(cache.get(&doc, &hash).is_none());
    }
    assert_eq!(cache.retained_entries(), 0);
    for i in 0..IMAGE_CACHE_MAX_ENTRIES * 4 {
        let hash = format!("{:040x}", i + 1_000_000);
        doc.images.insert(hash.clone(), png(2, i as u32));
        assert!(cache.get(&doc, &hash).is_some());
    }
    assert_eq!(cache.retained_entries(), IMAGE_CACHE_MAX_ENTRIES);
    let encoded = png(16, 7);
    doc.images.insert("source-bytes".into(), encoded.clone());
    assert!(cache.get(&doc, "source-bytes").is_some());
    assert!(cache.retained_bytes() >= 16 * 16 * 4 + encoded.len());
    let valid = png(2, 1);
    doc.images.insert("held".into(), valid.clone());
    let held = cache.get(&doc, "held").unwrap();
    for i in 0..(IMAGE_CACHE_MAX_ENTRIES + 1) {
        let hash = format!("{:040x}", i + 10000);
        doc.images.insert(hash.clone(), valid.clone());
        let _ = cache.get(&doc, &hash);
    }
    assert!(cache.retained_entries() <= IMAGE_CACHE_MAX_ENTRIES);
    assert_eq!(held.width(), 2, "eviction cannot invalidate a caller's Arc");
}

#[test]
fn a_document_without_the_bytes_does_not_poison_the_cache_for_one_that_has_them() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let hash = format!("{:040x}", 7u32);
    let mut with = Document::open(&path).unwrap();
    with.images.insert(hash.clone(), png(16, 7));
    let without = Document::open(&path).unwrap();

    let mut cache = figstudio_render::paint::ImageCache::default();
    assert!(
        cache.get(&without, &hash).is_none(),
        "no bytes, so no image"
    );
    assert!(
        cache.get(&with, &hash).is_some(),
        "a missing lookup must not cache a miss for the document that has the bytes"
    );
    assert!(
        cache.get(&without, &hash).is_none(),
        "a document without bytes never inherits a cached image"
    );

    let mut conflict = Document::open(&path).unwrap();
    conflict.images.insert(hash.clone(), png(16, 99));
    let first = cache.get(&with, &hash).unwrap();
    let second = cache.get(&conflict, &hash).unwrap();
    assert_ne!(
        first.data_as_u8_slice(),
        second.data_as_u8_slice(),
        "same hash with different bytes must not alias"
    );
}

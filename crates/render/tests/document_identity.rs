//! What a document registers in the shared image cache (its decoded images, its shader host)
//! belongs to the document, not to one revision of it: edits must not strand it, and closing
//! the document must free all of it.
mod common;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::*;
use figstudio_fig::{Document, Val};
use figstudio_render::images::{Images, cache_key};
use figstudio_render::shader::{Shaded, ShaderFills};
use figstudio_render::{Renderer, Viewport};
use vello_cpu::Pixmap;
use vello_cpu::kurbo::Affine;

/// The id the cache and the shader registry know `doc` by.
fn identity(doc: &Document) -> u64 {
    doc.lineage
}

/// Counts the fills it draws.
struct Counting(Arc<AtomicUsize>);

impl ShaderFills for Counting {
    fn fill(&self, _: &Document, _: &Val, _: u16, _: u16, _: f64) -> Shaded {
        self.0.fetch_add(1, Ordering::Relaxed);
        Shaded::Drawn(Arc::new(Pixmap::new(1, 1)))
    }
    fn has_effect(&self, _: &Document, _: &Val) -> bool {
        false
    }
    fn effect(&self, _: &Document, _: &Val, _: &Pixmap, _: f64) -> Shaded {
        Shaded::Unavailable
    }
}

fn shader_doc() -> Document {
    let mut rect = node(2, 1, "RECTANGLE");
    rect.data.set(
        "fillPaints",
        Val::Arr(vec![obj(&[
            ("type", Val::Str("CUSTOM".into())),
            ("visible", Val::Bool(true)),
        ])]),
    );
    document(vec![node(1, 0, "CANVAS"), rect])
}

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn draw(renderer: &mut Renderer, doc: &Document) {
    renderer.render(
        doc,
        id(1),
        &Viewport {
            width: 80,
            height: 80,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
}

#[test]
fn shader_fills_are_still_drawn_after_the_document_is_edited() {
    let images = Images::default();
    let mut doc = shader_doc();
    let calls = Arc::new(AtomicUsize::new(0));
    images.set_shaders(identity(&doc), Arc::new(Counting(calls.clone())));
    let mut renderer = Renderer::with_images(images);
    renderer.system_fonts = false;
    draw(&mut renderer, &doc);
    assert_eq!(calls.load(Ordering::Relaxed), 1, "control: the host draws");

    let nodes = doc.nodes.clone();
    doc.set_nodes(nodes).unwrap();
    draw(&mut renderer, &doc);
    assert_eq!(
        calls.load(Ordering::Relaxed),
        2,
        "the edited document lost its shader host"
    );
}

#[test]
fn closing_a_document_frees_the_images_cached_before_its_last_edit() {
    let images = Images::default();
    let mut doc = shader_doc();
    let nodes = doc.nodes.clone();
    doc.set_nodes(nodes).unwrap();
    assert!(images.insert_encoded(&cache_key(&doc, "ab"), &png(), 0));
    assert!(images.bytes() > 0);
    images.close_document(identity(&doc));
    assert_eq!(
        images.bytes(),
        0,
        "an edited document's images stayed cached"
    );
    assert!(
        !images.insert_encoded(&cache_key(&doc, "cd"), &png(), 0),
        "a closed document stored an image"
    );
}

#[test]
fn another_documents_images_and_shaders_are_not_touched_by_a_close() {
    let images = Images::default();
    let (a, b) = (shader_doc(), shader_doc());
    assert_ne!(identity(&a), identity(&b));
    images.insert_encoded(&cache_key(&a, "ab"), &png(), 0);
    images.insert_encoded(&cache_key(&b, "ab"), &png(), 0);
    images.close_document(identity(&a));
    assert_eq!(images.bytes(), 8 * 8 * 4, "only b's image is left");
}

#[test]
fn closing_a_document_rejects_late_shader_registration() {
    let images = Images::default();
    let doc = shader_doc();
    let calls = Arc::new(AtomicUsize::new(0));
    let host: Arc<dyn ShaderFills> = Arc::new(Counting(calls.clone()));
    images.close_document(identity(&doc));
    images.set_shaders(identity(&doc), host.clone());
    assert_eq!(
        Arc::strong_count(&host),
        1,
        "a closed lineage retained a late shader host"
    );
    let mut renderer = Renderer::with_images(images);
    renderer.system_fonts = false;
    draw(&mut renderer, &doc);
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "a closed lineage invoked a late shader host"
    );
}

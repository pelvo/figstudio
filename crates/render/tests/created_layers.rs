//! Layers made by the editor draw pixels, also after a save and reopen.

use std::path::PathBuf;
use std::sync::Arc;

use figstudio_fig::Document;
use figstudio_fig::edit::{EditOp, History, execute_edit_ops};
use figstudio_render::kurbo::Affine;
use figstudio_render::{Renderer, Viewport};

/// A fresh page of `structure.fig` holding only what `ops` create, saved and reopened.
fn page_with(
    ops: impl FnOnce(figstudio_fig::Guid) -> Vec<EditOp>,
) -> (Document, figstudio_fig::Guid) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let mut doc = Document::open(path).unwrap();
    let mut history = History::new(20);
    let page = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreatePage {
            id: None,
            name: "Created".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    execute_edit_ops(&mut doc, &mut history, &ops(page), None, None, false).unwrap();
    (
        Document::from_bytes(&doc.to_bytes().unwrap()).unwrap(),
        page,
    )
}

/// Pixels of `page` (page coordinates 0..200) that are clearly drawn.
fn drawn(doc: &Document, page: figstudio_fig::Guid) -> usize {
    let mut r = Renderer::new();
    r.add_font_data(Arc::new(epaint_default_fonts::HACK_REGULAR.to_vec()), 0);
    let frame = r.render(
        doc,
        page,
        &Viewport {
            width: 200,
            height: 200,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    frame
        .pixmap
        .data_as_u8_slice()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] > 100)
        .count()
}

#[test]
fn a_created_line_draws_its_default_stroke() {
    let (doc, page) = page_with(|page| {
        vec![EditOp::CreateShape {
            id: None,
            kind: "LINE".into(),
            parent: page,
            name: None,
            x: 20.0,
            y: 100.0,
            width: 160.0,
            height: 0.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }]
    });
    assert!(drawn(&doc, page) >= 160, "the line covers no pixels");
}

#[test]
fn created_text_draws_glyphs() {
    let (doc, page) = page_with(|page| {
        vec![EditOp::CreateText {
            id: None,
            parent: page,
            x: 20.0,
            y: 80.0,
            characters: "Hello edit".into(),
            font_family: "Hack".into(),
            font_style: "Regular".into(),
            font_size: 24.0,
            fill_hex: "#111827".into(),
        }]
    });
    assert!(drawn(&doc, page) > 100, "the text draws no glyphs");
}

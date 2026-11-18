//! Arrangement edits draw where the geometry says: grouping changes no pixel, a flip mirrors,
//! an align moves.

use std::path::PathBuf;

use figstudio_fig::edit::{AlignEdge, AlignTo, Axis, EditOp, History, execute_edit_ops};
use figstudio_fig::{Document, Guid};
use figstudio_render::kurbo::Affine;
use figstudio_render::{Renderer, Viewport};

fn shape(kind: &str, parent: Guid, x: f64, w: f64, hex: &str) -> EditOp {
    EditOp::CreateShape {
        id: None,
        kind: kind.into(),
        parent,
        name: None,
        x,
        y: 0.0,
        width: w,
        height: 50.0,
        corner_radius: 0.0,
        fill_hex: Some(hex.into()),
        count: None,
        star_inner_scale: None,
    }
}

fn run(doc: &mut Document, h: &mut History, ops: &[EditOp]) -> Vec<Guid> {
    execute_edit_ops(doc, h, ops, None, None, false)
        .unwrap()
        .0
        .created_ids
}

/// RGBA pixels of `page`, page coordinates 0..200 × 0..50.
fn pixels(doc: &Document, page: Guid) -> Vec<u8> {
    let frame = Renderer::new().render(
        doc,
        page,
        &Viewport {
            width: 200,
            height: 50,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    frame.pixmap.data_as_u8_slice().to_vec()
}

fn red_at(px: &[u8], x: usize) -> bool {
    let p = &px[(25 * 200 + x) * 4..][..4];
    p[0] > 200 && p[1] < 60 && p[3] > 200
}

#[test]
fn group_draws_the_same_flip_mirrors_and_align_moves() {
    let mut doc = Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
    )
    .unwrap();
    let mut h = History::new(20);
    let page = run(
        &mut doc,
        &mut h,
        &[EditOp::CreatePage {
            id: None,
            name: "P".into(),
        }],
    )[0];
    let frame = run(
        &mut doc,
        &mut h,
        &[shape("FRAME", page, 0.0, 100.0, "#FFFFFF")],
    )[0];
    let red = run(
        &mut doc,
        &mut h,
        &[shape("RECTANGLE", frame, 0.0, 20.0, "#FF0000")],
    )[0];
    let blue = run(
        &mut doc,
        &mut h,
        &[shape("RECTANGLE", page, 120.0, 20.0, "#0000FF")],
    )[0];

    let before = pixels(&doc, page);
    assert!(red_at(&before, 10) && !red_at(&before, 90));
    run(
        &mut doc,
        &mut h,
        &[EditOp::Group {
            targets: vec![frame, blue],
            name: None,
        }],
    );
    assert_eq!(pixels(&doc, page), before, "grouping changed pixels");

    run(
        &mut doc,
        &mut h,
        &[EditOp::Flip {
            target: frame,
            axis: Axis::Horizontal,
        }],
    );
    let flipped = pixels(&doc, page);
    assert!(
        !red_at(&flipped, 10) && red_at(&flipped, 90),
        "the flip did not mirror"
    );

    run(
        &mut doc,
        &mut h,
        &[EditOp::Align {
            targets: vec![red],
            edge: AlignEdge::HCenter,
            to: AlignTo::Parent,
        }],
    );
    let aligned = pixels(&doc, page);
    assert!(
        red_at(&aligned, 50) && !red_at(&aligned, 90),
        "align did not move"
    );
}

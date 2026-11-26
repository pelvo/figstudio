//! Booleans and outlined strokes made by the editor draw their computed result.

use std::path::PathBuf;

use figstudio_fig::edit::vector_net::BooleanOp;
use figstudio_fig::edit::{EditOp, History, execute_edit_ops};
use figstudio_fig::{Document, Guid};
use figstudio_render::kurbo::Affine;
use figstudio_render::{Renderer, Viewport};

fn run(doc: &mut Document, h: &mut History, ops: Vec<EditOp>) -> Vec<Guid> {
    execute_edit_ops(doc, h, &ops, None, None, false)
        .unwrap()
        .0
        .created_ids
}

fn rect(parent: Guid, x: f64, y: f64, w: f64, h: f64) -> EditOp {
    EditOp::CreateShape {
        id: None,
        kind: "RECTANGLE".into(),
        parent,
        name: None,
        x,
        y,
        width: w,
        height: h,
        corner_radius: 0.0,
        fill_hex: Some("#FF0000".into()),
        count: None,
        star_inner_scale: None,
    }
}

#[test]
fn a_subtract_boolean_draws_a_hole_and_follows_an_operand_move() {
    let mut doc = Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
    )
    .unwrap();
    let mut h = History::new(20);
    let page = run(
        &mut doc,
        &mut h,
        vec![EditOp::CreatePage {
            id: None,
            name: "P".into(),
        }],
    )[0];
    let outer = run(&mut doc, &mut h, vec![rect(page, 0.0, 0.0, 200.0, 200.0)])[0];
    let hole = run(&mut doc, &mut h, vec![rect(page, 50.0, 50.0, 100.0, 100.0)])[0];
    run(
        &mut doc,
        &mut h,
        vec![EditOp::BooleanCombine {
            operation: BooleanOp::Subtract,
            targets: vec![outer, hole],
            name: None,
        }],
    );
    let alpha = |doc: &Document, x: usize, y: usize| {
        let frame = Renderer::new().render(
            doc,
            page,
            &Viewport {
                width: 200,
                height: 200,
                transform: Affine::IDENTITY,
                background: None,
            },
        );
        frame.pixmap.data_as_u8_slice()[(y * 200 + x) * 4 + 3]
    };
    assert!(alpha(&doc, 20, 100) > 200, "the ring is drawn");
    assert!(alpha(&doc, 100, 100) < 20, "the hole is empty");
    run(
        &mut doc,
        &mut h,
        vec![EditOp::Move {
            target: hole,
            x: 300.0,
            y: 300.0,
            relative: false,
        }],
    );
    assert!(alpha(&doc, 100, 100) > 200, "the hole moved away");
}

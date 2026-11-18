//! Drag math under a rotated parent: the layer follows the pointer in page space.

use figstudio_fig::edit::{EditOp, History, execute_edit_ops};
use figstudio_render::kurbo::Point;

use super::*;

fn shape(kind: &str, parent: Guid, x: f64, y: f64, w: f64, h: f64) -> EditOp {
    EditOp::CreateShape {
        id: None,
        kind: kind.into(),
        parent,
        name: None,
        x,
        y,
        width: w,
        height: h,
        corner_radius: 0.0,
        fill_hex: None,
        count: None,
        star_inner_scale: None,
    }
}

/// A rectangle inside a frame rotated 90°.
fn rotated_parent() -> (Document, History, Guid) {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let mut doc = Document::open(path).unwrap();
    let mut history = History::new(20);
    let page = doc.pages[0];
    let run = |doc: &mut Document, history: &mut History, op: EditOp| {
        execute_edit_ops(doc, history, &[op], None, None, false)
            .unwrap()
            .0
            .created_ids
    };
    let frame = run(
        &mut doc,
        &mut history,
        shape("FRAME", page, 400.0, 400.0, 200.0, 200.0),
    )[0];
    run(
        &mut doc,
        &mut history,
        EditOp::Rotate {
            target: frame,
            degrees: 90.0,
            pivot: Default::default(),
        },
    );
    let rect = run(
        &mut doc,
        &mut history,
        shape("RECTANGLE", frame, 10.0, 10.0, 40.0, 20.0),
    )[0];
    (doc, history, rect)
}

fn page_point(doc: &Document, id: Guid, local: Point) -> Point {
    absolute_transform(doc, id) * local
}

fn apply(doc: &mut Document, history: &mut History, ops: &[EditOp]) {
    execute_edit_ops(doc, history, ops, None, None, false).unwrap();
}

fn assert_near(a: Point, b: Point) {
    assert!((a - b).hypot() < 1e-3, "{a:?} != {b:?}");
}

#[test]
fn move_under_a_rotated_parent_follows_the_pointer_in_page_space() {
    let (mut doc, mut history, rect) = rotated_parent();
    let before = page_point(&doc, rect, Point::ZERO);
    let (ox, oy, ..) = crate::editor::local_xywh(&doc.node(rect).unwrap().data);
    let (x, y) = moved_to(&doc, rect, ox, oy, 20.0, 0.0);
    apply(
        &mut doc,
        &mut history,
        &[EditOp::Move {
            target: rect,
            x,
            y,
            relative: false,
        }],
    );
    assert_near(
        page_point(&doc, rect, Point::ZERO),
        before + figstudio_render::kurbo::Vec2::new(20.0, 0.0),
    );
}

#[test]
fn resize_under_a_rotated_parent_uses_the_layer_axes_and_keeps_the_opposite_edge() {
    for (handle, keep) in [(Handle::E, Point::ZERO), (Handle::W, Point::new(40.0, 0.0))] {
        let (mut doc, mut history, rect) = rotated_parent();
        let orig = crate::editor::local_xywh(&doc.node(rect).unwrap().data);
        let kept = page_point(&doc, rect, keep);
        // The layer's own x axis in page space: dragging along it by 10 changes the width by 10.
        let axis =
            page_point(&doc, rect, Point::new(1.0, 0.0)) - page_point(&doc, rect, Point::ZERO);
        let sign = if handle == Handle::E { 10.0 } else { -10.0 };
        let (x, y, w, h) = resized_to(&doc, rect, handle, orig, axis.x * sign, axis.y * sign);
        assert!(
            (w - 50.0).abs() < 1e-6 && (h - 20.0).abs() < 1e-6,
            "{handle:?}: {w}x{h}"
        );
        apply(
            &mut doc,
            &mut history,
            &[
                EditOp::Move {
                    target: rect,
                    x,
                    y,
                    relative: false,
                },
                EditOp::Resize {
                    target: rect,
                    width: w,
                    height: h,
                },
            ],
        );
        let kept_after = if handle == Handle::E {
            Point::ZERO
        } else {
            Point::new(50.0, 0.0)
        };
        assert_near(page_point(&doc, rect, kept_after), kept);
    }
}

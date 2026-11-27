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

#[test]
fn rotated_handle_cursors_follow_screen_orientation() {
    let rot90 = Affine::rotate(std::f64::consts::FRAC_PI_2);
    assert_eq!(Handle::E.cursor_for(rot90), CursorIcon::ResizeVertical);
    assert_eq!(Handle::W.cursor_for(rot90), CursorIcon::ResizeVertical);
    assert_eq!(Handle::N.cursor_for(rot90), CursorIcon::ResizeHorizontal);
    assert_eq!(Handle::S.cursor_for(rot90), CursorIcon::ResizeHorizontal);
    assert_eq!(Handle::NW.cursor_for(rot90), CursorIcon::ResizeNeSw);
    assert_eq!(Handle::NE.cursor_for(rot90), CursorIcon::ResizeNwSe);

    let rot45 = Affine::rotate(std::f64::consts::FRAC_PI_4);
    assert_eq!(Handle::E.cursor_for(rot45), CursorIcon::ResizeNwSe);
    assert_eq!(Handle::S.cursor_for(rot45), CursorIcon::ResizeNeSw);
    assert_eq!(Handle::SE.cursor_for(rot45), CursorIcon::ResizeVertical);
    assert_eq!(Handle::NE.cursor_for(rot45), CursorIcon::ResizeHorizontal);
}

fn set_matrix(doc: &mut Document, id: Guid, m: [f64; 6]) {
    let mut nodes = doc.nodes.clone();
    let n = nodes.iter_mut().find(|n| n.id == id).expect("node exists");
    let tf = n.data.get_mut("transform").expect("transform field");
    for (k, v) in [
        ("m00", m[0]),
        ("m01", m[1]),
        ("m02", m[2]),
        ("m10", m[3]),
        ("m11", m[4]),
        ("m12", m[5]),
    ] {
        tf.set(k, figstudio_fig::Val::Float(v));
    }
    doc.set_nodes(nodes).unwrap();
}

#[test]
fn resize_preserves_opposite_edge_under_45deg_skew_and_nonuniform_transformed_parent() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");

    type Mat = [f64; 6];
    let cases: [(&str, Option<Mat>, Mat); 3] = [
        // 45° rotated node
        (
            "45deg",
            None,
            [
                std::f64::consts::FRAC_1_SQRT_2,
                -std::f64::consts::FRAC_1_SQRT_2,
                100.0,
                std::f64::consts::FRAC_1_SQRT_2,
                std::f64::consts::FRAC_1_SQRT_2,
                120.0,
            ],
        ),
        // Skewed node (m01 != 0)
        ("skewed", None, [1.0, 0.5, 80.0, -0.25, 1.2, 60.0]),
        // Node inside non-uniformly scaled & skewed parent
        (
            "nonuniform-skewed-parent",
            Some([2.0, 0.6, 200.0, -0.3, 0.5, 150.0]),
            [0.8, -0.4, 15.0, 0.3, 1.1, 25.0],
        ),
    ];

    for (label, parent_mat, node_mat) in cases {
        for handle in [Handle::W, Handle::N, Handle::NW] {
            for snap_size in [false, true] {
                let mut doc = Document::open(&path).unwrap();
                let mut history = History::new(20);
                let page = doc.pages[0];
                let parent = if let Some(pm) = parent_mat {
                    let p = execute_edit_ops(
                        &mut doc,
                        &mut history,
                        &[shape("FRAME", page, 0.0, 0.0, 300.0, 300.0)],
                        None,
                        None,
                        false,
                    )
                    .unwrap()
                    .0
                    .created_ids[0];
                    set_matrix(&mut doc, p, pm);
                    p
                } else {
                    page
                };
                let rect = execute_edit_ops(
                    &mut doc,
                    &mut history,
                    &[shape("RECTANGLE", parent, 0.0, 0.0, 80.0, 50.0)],
                    None,
                    None,
                    false,
                )
                .unwrap()
                .0
                .created_ids[0];
                set_matrix(&mut doc, rect, node_mat);

                let orig = crate::editor::local_xywh(&doc.node(rect).unwrap().data);
                let opp_before = page_point(&doc, rect, Point::new(orig.2, orig.3));
                let (x, y, w, h) = resized_with(&doc, rect, handle, orig, -13.7, -9.3, snap_size);
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
                let opp_after = page_point(&doc, rect, Point::new(w, h));
                assert!(
                    (opp_after - opp_before).hypot() < 1e-3,
                    "{label} {handle:?} snap={snap_size}: SE corner drifted from {opp_before:?} to {opp_after:?}"
                );
            }
        }
    }

    // Singular parent (zero-scale X): resized_with must still read the node's own local matrix
    // instead of falling back to singular `P * L` and zeroing out the shift.
    let mut doc = Document::open(&path).unwrap();
    let mut history = History::new(20);
    let page = doc.pages[0];
    let parent = execute_edit_ops(
        &mut doc,
        &mut history,
        &[shape("FRAME", page, 0.0, 0.0, 200.0, 200.0)],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    set_matrix(&mut doc, parent, [0.0, 0.0, 100.0, 0.0, 1.0, 100.0]);
    let rect = execute_edit_ops(
        &mut doc,
        &mut history,
        &[shape("RECTANGLE", parent, 20.0, 30.0, 80.0, 40.0)],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let orig = crate::editor::local_xywh(&doc.node(rect).unwrap().data);
    let (x, _y, w, _h) = resized_with(&doc, rect, Handle::W, orig, -10.0, 0.0, false);
    assert!(
        (w - 90.0).abs() < 1e-6 && (x - 10.0).abs() < 1e-6,
        "singular parent must still use node's local matrix for W shift: got x={x}, w={w}"
    );
}

#[test]
fn non_drag_tab_edits_end_drag_and_cancel_checks_history_ownership() {
    use std::sync::Arc;
    use std::time::Duration;

    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let doc = Arc::new(Document::open(&path).unwrap());
    let mut tab = Tab::new(1, path, doc, None, Duration::ZERO);
    let node_a = Guid(1, 34);
    let node_b = Guid(1, 25);
    tab.select(Some(node_a));

    // 1. Direct history-ownership guard: if another mutation advances history without clearing
    // edit_drag, cancel_edit_drag must not undo that unrelated transaction.
    let (ox, oy, _, _) = crate::editor::local_xywh(&tab.doc.node(node_a).unwrap().data);
    let mut drag = CanvasDrag::Move {
        target: node_a,
        start_pointer_page: pos2(0.0, 0.0),
        orig_x: ox,
        orig_y: oy,
        moved: false,
        last_applied: None,
        drag_state_id: None,
    };
    let mut cursor = None;
    assert!(step_edit_drag(
        &mut tab,
        &mut drag,
        pos2(20.0, 0.0),
        6.0,
        &mut cursor
    ));
    tab.edit_drag = Some(drag);
    crate::editor::apply_tab_ops(
        &mut tab,
        &[EditOp::Rename {
            target: node_b,
            name: "external-edit".into(),
        }],
        Some("Rename"),
        None,
    )
    .unwrap();
    assert!(cancel_edit_drag(&mut tab));
    assert!(tab.edit_drag.is_none());
    assert_eq!(tab.doc.node(node_b).unwrap().name, "external-edit");

    // 2. Non-drag UI helpers (duplicate, delete, paste, undo, redo) end an active drag on commit.
    let arm_move_drag = |tab: &mut Tab, target: Guid| {
        let (x, y, _, _) = crate::editor::local_xywh(&tab.doc.node(target).unwrap().data);
        let mut d = CanvasDrag::Move {
            target,
            start_pointer_page: pos2(0.0, 0.0),
            orig_x: x,
            orig_y: y,
            moved: false,
            last_applied: None,
            drag_state_id: None,
        };
        let mut c = None;
        assert!(step_edit_drag(tab, &mut d, pos2(12.0, 0.0), 6.0, &mut c));
        tab.edit_drag = Some(d);
    };

    arm_move_drag(&mut tab, node_a);
    crate::editor::duplicate_selection(&mut tab).unwrap();
    assert!(
        tab.edit_drag.is_none(),
        "duplicate_selection ends active drag"
    );

    let dup_id = tab.selection.unwrap();
    arm_move_drag(&mut tab, dup_id);
    crate::editor::delete_selection(&mut tab).unwrap();
    assert!(tab.edit_drag.is_none(), "delete_selection ends active drag");

    tab.select(Some(node_a));
    arm_move_drag(&mut tab, node_a);
    let html = figstudio_fig::clipboard::encode_clipboard_html(&tab.doc, &[node_b], 1, None, None)
        .unwrap();
    crate::editor::paste_from_clipboard_or_text(&mut tab, Some(&html)).unwrap();
    assert!(
        tab.edit_drag.is_none(),
        "paste_from_clipboard_or_text ends active drag"
    );

    tab.select(Some(node_a));
    arm_move_drag(&mut tab, node_a);
    assert!(crate::editor::undo_tab(&mut tab).unwrap().is_some());
    assert!(tab.edit_drag.is_none(), "undo_tab ends active drag");

    arm_move_drag(&mut tab, node_a);
    // Re-seed redo stack by undoing once before testing redo_tab ends an active drag.
    assert!(crate::editor::undo_tab(&mut tab).unwrap().is_some());
    let (x, y, _, _) = crate::editor::local_xywh(&tab.doc.node(node_a).unwrap().data);
    tab.edit_drag = Some(CanvasDrag::Move {
        target: node_a,
        start_pointer_page: pos2(0.0, 0.0),
        orig_x: x,
        orig_y: y,
        moved: false,
        last_applied: None,
        drag_state_id: None,
    });
    assert!(crate::editor::redo_tab(&mut tab).unwrap().is_some());
    assert!(tab.edit_drag.is_none(), "redo_tab ends active drag");
}

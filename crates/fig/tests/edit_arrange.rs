//! Arrangement ops: group, ungroup, frame selection, align, distribute, flip, rotate about the
//! centre, z-order.

use std::path::PathBuf;

use figstudio_fig::edit::space::{Bounds, layer_bounds, world_transform};
use figstudio_fig::edit::{
    AlignEdge, AlignTo, Axis, EditOp, History, Pivot, ZOrder, execute_edit_ops,
};
use figstudio_fig::{Document, Guid};

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

struct Fx {
    doc: Document,
    h: History,
    page: Guid,
}

impl Fx {
    /// A new empty page of `structure.fig`.
    fn new() -> Fx {
        let mut doc = Document::open(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
        )
        .unwrap();
        let mut h = History::new(50);
        let page = execute_edit_ops(
            &mut doc,
            &mut h,
            &[EditOp::CreatePage {
                id: None,
                name: "Arrange".into(),
            }],
            None,
            None,
            false,
        )
        .unwrap()
        .0
        .created_ids[0];
        Fx { doc, h, page }
    }

    fn run(&mut self, ops: &[EditOp]) -> Vec<Guid> {
        execute_edit_ops(&mut self.doc, &mut self.h, ops, None, None, false)
            .unwrap()
            .0
            .created_ids
    }

    fn try_run(&mut self, ops: &[EditOp]) -> bool {
        execute_edit_ops(&mut self.doc, &mut self.h, ops, None, None, false).is_ok()
    }

    fn b(&self, id: Guid) -> Bounds {
        layer_bounds(&self.doc, id, self.page).unwrap()
    }

    /// Three rectangles of different widths on the page.
    fn three(&mut self) -> Vec<Guid> {
        let page = self.page;
        self.run(&[
            shape("RECTANGLE", page, 0.0, 0.0, 20.0, 10.0),
            shape("RECTANGLE", page, 50.0, 30.0, 40.0, 10.0),
            shape("RECTANGLE", page, 200.0, 60.0, 10.0, 10.0),
        ])
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

fn same(a: Bounds, b: Bounds) -> bool {
    near(a.x0, b.x0) && near(a.y0, b.y0) && near(a.x1, b.x1) && near(a.y1, b.y1)
}

#[test]
fn group_wraps_siblings_in_place_and_ungroup_restores_them() {
    let mut f = Fx::new();
    let r = f.three();
    let before: Vec<Bounds> = r.iter().map(|&id| f.b(id)).collect();
    let depth = f.h.undo_len();
    let g = f.run(&[EditOp::Group {
        targets: vec![r[0], r[1]],
        name: None,
    }])[0];
    assert_eq!(f.h.undo_len(), depth + 1);
    let node = f.doc.node(g).unwrap();
    assert_eq!(node.kind, "FRAME");
    assert_eq!(node.data.flag("resizeToFit"), Some(true));
    assert_eq!(f.doc.children(g), &[r[0], r[1]]);
    // The group took the topmost target's slot: below the third rectangle.
    assert_eq!(f.doc.children(f.page), &[g, r[2]]);
    for (id, b) in r.iter().zip(&before) {
        assert!(same(f.b(*id), *b), "a grouped layer moved");
    }
    assert!(same(f.b(g), before[0].union(before[1])));

    f.run(&[EditOp::Ungroup { target: g }]);
    assert!(f.doc.node(g).is_none());
    assert_eq!(f.doc.children(f.page), &[r[0], r[1], r[2]]);
    for (id, b) in r.iter().zip(&before) {
        assert!(same(f.b(*id), *b), "an ungrouped layer moved");
    }
}

#[test]
fn frame_selection_makes_a_real_frame_and_grouping_checks_its_targets() {
    let mut f = Fx::new();
    let r = f.three();
    let fr = f.run(&[EditOp::FrameSelection {
        targets: vec![r[1], r[2]],
        name: None,
    }])[0];
    let node = f.doc.node(fr).unwrap();
    assert_eq!(node.kind, "FRAME");
    assert_eq!(node.data.flag("resizeToFit"), None);
    assert!(f.doc.children(fr).contains(&r[1]));
    // Repeated, mixed-parent and empty target lists are refused; so is ungrouping a frame.
    assert!(!f.try_run(&[EditOp::Group {
        targets: vec![r[0], r[0]],
        name: None
    }]));
    assert!(!f.try_run(&[EditOp::Group {
        targets: vec![r[0], r[1]],
        name: None
    }]));
    assert!(!f.try_run(&[EditOp::Group {
        targets: vec![],
        name: None
    }]));
    assert!(!f.try_run(&[EditOp::Ungroup { target: fr }]));
}

#[test]
fn align_lines_up_edges_of_the_selection_or_the_parent() {
    let mut f = Fx::new();
    let r = f.three();
    f.run(&[EditOp::Align {
        targets: r.clone(),
        edge: AlignEdge::Right,
        to: AlignTo::Selection,
    }]);
    for &id in &r {
        assert!(near(f.b(id).x1, 210.0));
    }
    f.run(&[EditOp::Align {
        targets: r.clone(),
        edge: AlignEdge::VCenter,
        to: AlignTo::Selection,
    }]);
    for &id in &r {
        let b = f.b(id);
        assert!(near((b.y0 + b.y1) / 2.0, 35.0));
    }
    // One layer aligns to its frame; on the page there is nothing to align to.
    let page = f.page;
    let frame = f.run(&[shape("FRAME", page, 300.0, 300.0, 100.0, 100.0)])[0];
    let inner = f.run(&[shape("RECTANGLE", frame, 5.0, 5.0, 10.0, 10.0)])[0];
    f.run(&[EditOp::Align {
        targets: vec![inner],
        edge: AlignEdge::Bottom,
        to: AlignTo::Selection,
    }]);
    assert!(near(f.b(inner).y1, 400.0));
    assert!(!f.try_run(&[EditOp::Align {
        targets: vec![r[0]],
        edge: AlignEdge::Left,
        to: AlignTo::Selection,
    }]));
}

#[test]
fn distribute_spaces_layers_evenly_or_by_a_fixed_gap() {
    let mut f = Fx::new();
    let r = f.three();
    f.run(&[EditOp::Distribute {
        targets: r.clone(),
        axis: Axis::Horizontal,
        spacing: None,
    }]);
    let (a, b, c) = (f.b(r[0]), f.b(r[1]), f.b(r[2]));
    assert!(near(b.x0 - a.x1, c.x0 - b.x1), "gaps differ");
    assert!(near(a.x0, 0.0) && near(c.x1, 210.0), "the ends moved");
    f.run(&[EditOp::Distribute {
        targets: vec![r[2], r[0]],
        axis: Axis::Vertical,
        spacing: Some(5.0),
    }]);
    assert!(near(f.b(r[2]).y0 - f.b(r[0]).y1, 5.0));
    assert!(!f.try_run(&[EditOp::Distribute {
        targets: vec![r[0], r[1]],
        axis: Axis::Horizontal,
        spacing: None,
    }]));
}

#[test]
fn flip_mirrors_in_place_and_rotate_about_the_centre_keeps_the_centre() {
    let mut f = Fx::new();
    let r = f.three();
    let before = f.b(r[1]);
    let t0 = world_transform(&f.doc, r[1]);
    f.run(&[EditOp::Flip {
        target: r[1],
        axis: Axis::Horizontal,
    }]);
    assert!(same(f.b(r[1]), before));
    assert!(world_transform(&f.doc, r[1]).determinant() < 0.0);
    f.run(&[EditOp::Flip {
        target: r[1],
        axis: Axis::Horizontal,
    }]);
    assert_eq!(world_transform(&f.doc, r[1]), t0);

    let c = |b: Bounds| ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
    let centre = c(f.b(r[1]));
    f.run(&[EditOp::Rotate {
        target: r[1],
        degrees: 90.0,
        pivot: Pivot::Center,
    }]);
    let after = f.b(r[1]);
    assert!(near(c(after).0, centre.0) && near(c(after).1, centre.1));
    assert!(near(after.width(), 10.0) && near(after.height(), 40.0));
}

#[test]
fn reorder_moves_a_layer_through_its_siblings() {
    let mut f = Fx::new();
    let r = f.three();
    f.run(&[EditOp::Reorder {
        target: r[0],
        to: ZOrder::Front,
    }]);
    assert_eq!(f.doc.children(f.page), &[r[1], r[2], r[0]]);
    f.run(&[EditOp::Reorder {
        target: r[0],
        to: ZOrder::Backward,
    }]);
    assert_eq!(f.doc.children(f.page), &[r[1], r[0], r[2]]);
    f.run(&[EditOp::Reorder {
        target: r[2],
        to: ZOrder::Back,
    }]);
    assert_eq!(f.doc.children(f.page), &[r[2], r[1], r[0]]);
    f.run(&[EditOp::Reorder {
        target: r[2],
        to: ZOrder::Forward,
    }]);
    assert_eq!(f.doc.children(f.page), &[r[1], r[2], r[0]]);
}

#[test]
fn reparent_can_keep_the_page_position_under_a_rotated_frame() {
    let mut f = Fx::new();
    let page = f.page;
    let ids = f.run(&[
        shape("FRAME", page, 100.0, 100.0, 200.0, 200.0),
        shape("RECTANGLE", page, 10.0, 20.0, 30.0, 40.0),
    ]);
    let (frame, rect) = (ids[0], ids[1]);
    f.run(&[EditOp::Rotate {
        target: frame,
        degrees: 30.0,
        pivot: Default::default(),
    }]);
    let before = f.b(rect);
    f.run(&[EditOp::Reparent {
        target: rect,
        new_parent: frame,
        index: None,
        keep_position: true,
    }]);
    assert_eq!(f.doc.children(frame), &[rect]);
    assert!(same(f.b(rect), before), "{:?} {before:?}", f.b(rect));
    // Back onto the page: still in place, and unrotated again.
    f.run(&[EditOp::Reparent {
        target: rect,
        new_parent: page,
        index: Some(0),
        keep_position: true,
    }]);
    assert_eq!(f.doc.children(page)[0], rect);
    assert!(same(f.b(rect), before));
    // Without the flag the local transform is kept, so it lands elsewhere.
    f.run(&[EditOp::Reparent {
        target: rect,
        new_parent: frame,
        index: None,
        keep_position: false,
    }]);
    assert!(!same(f.b(rect), before));
}

#[test]
fn arrangement_ops_round_trip_through_json() {
    let op: EditOp = serde_json::from_value(serde_json::json!({
        "op": "align", "targets": ["1:2", "1:3"], "edge": "h_center"
    }))
    .unwrap();
    assert!(matches!(
        op,
        EditOp::Align {
            to: AlignTo::Selection,
            ..
        }
    ));
    let op: EditOp = serde_json::from_value(serde_json::json!({
        "op": "rotate", "target": "1:2", "degrees": 45.0
    }))
    .unwrap();
    assert!(matches!(
        op,
        EditOp::Rotate {
            pivot: Pivot::TopLeft,
            ..
        }
    ));
}

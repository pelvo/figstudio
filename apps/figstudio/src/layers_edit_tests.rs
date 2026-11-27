//! Range selection over the listed rows and the `Reparent` ops a drop makes.

use figstudio_fig::edit::{EditOp, History, execute_edit_ops};

use super::*;

/// A page with rectangles `a`, `b`, `c` (back to front) and a frame holding `d`.
fn page() -> (Document, History, Guid, [Guid; 5]) {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let mut doc = Document::open(path).unwrap();
    let mut h = History::new(20);
    let shape = |kind: &str, parent: Guid, x: f64| EditOp::CreateShape {
        id: None,
        kind: kind.into(),
        parent,
        name: None,
        x,
        y: 0.0,
        width: 40.0,
        height: 40.0,
        corner_radius: 0.0,
        fill_hex: None,
        count: None,
        star_inner_scale: None,
    };
    let run = |doc: &mut Document, h: &mut History, ops: &[EditOp]| {
        execute_edit_ops(doc, h, ops, None, None, false)
            .unwrap()
            .0
            .created_ids
    };
    let pg = run(
        &mut doc,
        &mut h,
        &[EditOp::CreatePage {
            id: None,
            name: "L".into(),
        }],
    )[0];
    let ids = run(
        &mut doc,
        &mut h,
        &[
            shape("RECTANGLE", pg, 0.0),
            shape("RECTANGLE", pg, 50.0),
            shape("RECTANGLE", pg, 100.0),
            shape("FRAME", pg, 200.0),
        ],
    );
    let d = run(&mut doc, &mut h, &[shape("RECTANGLE", ids[3], 210.0)])[0];
    (doc, h, pg, [ids[0], ids[1], ids[2], ids[3], d])
}

fn apply(doc: &mut Document, h: &mut History, ops: &[EditOp]) {
    assert!(!ops.is_empty(), "the drop makes ops");
    execute_edit_ops(doc, h, ops, None, None, false).unwrap();
}

fn row(id: Guid, i: usize) -> Row {
    Row {
        id,
        depth: 0,
        rect: Rect::from_min_size(egui::pos2(0.0, i as f32 * 24.0), vec2(200.0, 24.0)),
    }
}

#[test]
fn shift_ranges_run_either_way_and_keep_the_anchor_first() {
    let ids: Vec<Guid> = (1..=5).map(|i| Guid(1, i)).collect();
    let rows: Vec<Row> = ids.iter().enumerate().map(|(i, g)| row(*g, i)).collect();
    assert_eq!(range(&rows, ids[1], ids[3]), vec![ids[1], ids[2], ids[3]]);
    assert_eq!(range(&rows, ids[3], ids[1]), vec![ids[3], ids[1], ids[2]]);
}

#[test]
fn rows_reorder_among_siblings_keeping_their_order() {
    let (mut doc, mut h, pg, [a, b, c, f, _]) = page();
    let never = |_| false;
    // Drag `a` (at the back) above `c` (in front).
    let ops = drop_ops(&doc, &[a], Drop::Above(c), &never);
    apply(&mut doc, &mut h, &ops);
    assert_eq!(doc.children(pg), &[b, c, a, f]);
    // Drag `a` and `c` (front first, as listed) below `b`: behind it, `a` still in front of `c`.
    let ops = drop_ops(&doc, &[a, c], Drop::Below(b), &never);
    apply(&mut doc, &mut h, &ops);
    assert_eq!(doc.children(pg), &[c, a, b, f]);
    // Dropping where they already are changes nothing.
    assert!(drop_ops(&doc, &[a], Drop::Above(c), &never).is_empty());
}

#[test]
fn reordering_in_a_reverse_stack_matches_the_visible_order() {
    let (mut doc, mut h, pg, [a, b, c, f, _]) = page();
    let mut txn = figstudio_fig::edit::Txn::new("Reverse stack order");
    txn.set_field(pg, "stackReverseZIndex", figstudio_fig::Val::Bool(true));
    h.execute(&mut doc, txn).unwrap();

    let ops = drop_ops(&doc, &[b, c], Drop::Above(a), &|_| false);
    apply(&mut doc, &mut h, &ops);
    assert_eq!(doc.children(pg), &[b, c, a, f]);
}

#[test]
fn rows_reparent_into_a_frame_and_stay_put_on_the_page() {
    let (mut doc, mut h, pg, [a, _, _, f, d]) = page();
    let before = figstudio_render::node_bounds(&doc, a).unwrap();
    let ops = drop_ops(&doc, &[a], Drop::Into(f), &|_| false);
    apply(&mut doc, &mut h, &ops);
    // At the top of the frame (in front of `d`), where it was on the page.
    assert_eq!(doc.children(f), &[d, a]);
    assert!(!doc.children(pg).contains(&a));
    assert_eq!(figstudio_render::node_bounds(&doc, a).unwrap(), before);
    // Below an open frame means inside it, at the top.
    let ops = drop_ops(&doc, &[d], Drop::Below(f), &|g| g == f);
    apply(&mut doc, &mut h, &ops);
    assert_eq!(doc.children(f), &[a, d]);
}

#[test]
fn a_layer_cannot_go_into_itself_or_its_own_layers() {
    let (doc, _, _, [_, _, _, f, d]) = page();
    assert!(drop_ops(&doc, &[f], Drop::Into(f), &|_| false).is_empty());
    assert!(drop_ops(&doc, &[f], Drop::Above(d), &|_| false).is_empty());
}

#[test]
fn the_drop_zone_depends_on_where_in_the_row_the_pointer_is() {
    let (doc, _, _, [a, _, _, f, _]) = page();
    let rows = [row(f, 0), row(a, 1)];
    // A frame: its top quarter is above, the middle is inside, the bottom quarter below.
    assert_eq!(drop_at(&doc, &rows, &[a], 2.0).unwrap().0, Drop::Above(f));
    assert_eq!(drop_at(&doc, &rows, &[a], 12.0).unwrap().0, Drop::Into(f));
    assert_eq!(drop_at(&doc, &rows, &[a], 22.0).unwrap().0, Drop::Below(f));
    // A rectangle takes nothing inside: halves.
    assert_eq!(drop_at(&doc, &rows, &[f], 40.0).unwrap().0, Drop::Below(a));
    assert_eq!(drop_at(&doc, &rows, &[f], 26.0).unwrap().0, Drop::Above(a));
}

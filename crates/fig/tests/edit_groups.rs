//! Groups are resize-to-fit frames that stay fitted to their children after any edit.

use std::path::PathBuf;

use figstudio_fig::edit::space::{layer_bounds, world_transform};
use figstudio_fig::edit::{EditOp, History, execute_edit_ops};
use figstudio_fig::{Document, Guid};

fn doc() -> Document {
    Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
    )
    .unwrap()
}

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

fn run(doc: &mut Document, h: &mut History, ops: &[EditOp]) -> Vec<Guid> {
    execute_edit_ops(doc, h, ops, None, None, false)
        .unwrap()
        .0
        .created_ids
}

fn page_origin(doc: &Document, id: Guid) -> (f64, f64) {
    world_transform(doc, id).apply((0.0, 0.0))
}

fn assert_near(a: (f64, f64), b: (f64, f64)) {
    assert!(
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3,
        "{a:?} != {b:?}"
    );
}

/// A group at (100, 100) holding two rectangles, as one batch made it.
fn grouped() -> (Document, History, Guid, Guid, Guid) {
    let mut d = doc();
    let page = d.pages[0];
    let mut h = History::new(20);
    let group = run(
        &mut d,
        &mut h,
        &[shape("GROUP", page, 100.0, 100.0, 1.0, 1.0)],
    )[0];
    let kids = run(
        &mut d,
        &mut h,
        &[
            shape("RECTANGLE", group, 10.0, 20.0, 50.0, 40.0),
            shape("RECTANGLE", group, 80.0, 5.0, 30.0, 30.0),
        ],
    );
    (d, h, group, kids[0], kids[1])
}

#[test]
fn a_group_is_stored_as_a_resize_to_fit_frame_that_fits_its_children() {
    let (d, _, group, a, b) = grouped();
    let node = d.node(group).unwrap();
    assert_eq!(node.kind, "FRAME");
    assert_eq!(node.data.flag("resizeToFit"), Some(true));
    // Children span x 10..110, y 5..60 inside the old origin: the group moved there.
    let page = d.pages[0];
    let g = layer_bounds(&d, group, page).unwrap();
    assert_near((g.x0, g.y0), (110.0, 105.0));
    assert_near((g.width(), g.height()), (100.0, 55.0));
    assert_near(page_origin(&d, a), (110.0, 120.0));
    assert_near(page_origin(&d, b), (180.0, 105.0));
    let reopened = Document::from_bytes(&d.to_bytes().unwrap()).unwrap();
    assert_eq!(
        reopened.node(group).unwrap().data.flag("resizeToFit"),
        Some(true)
    );
}

#[test]
fn moving_a_child_refits_the_group_keeps_page_positions_and_undoes_in_one_step() {
    let (mut d, mut h, group, a, b) = grouped();
    let page = d.pages[0];
    let before_b = page_origin(&d, b);
    let before_group = layer_bounds(&d, group, page).unwrap();
    let undo_depth = h.undo_len();
    run(
        &mut d,
        &mut h,
        &[EditOp::Move {
            target: a,
            x: -50.0,
            y: 0.0,
            relative: true,
        }],
    );
    assert_eq!(h.undo_len(), undo_depth + 1);
    assert_near(page_origin(&d, a), (60.0, 120.0));
    assert_near(page_origin(&d, b), before_b);
    let g = layer_bounds(&d, group, page).unwrap();
    assert_near((g.x0, g.y0), (60.0, 105.0));
    assert_near((g.width(), g.height()), (150.0, 55.0));
    h.undo(&mut d).unwrap();
    let g = layer_bounds(&d, group, page).unwrap();
    assert_near((g.x0, g.y0), (before_group.x0, before_group.y0));
    assert_near(
        (g.width(), g.height()),
        (before_group.width(), before_group.height()),
    );
    assert_near(page_origin(&d, a), (110.0, 120.0));
}

#[test]
fn nested_groups_refit_inner_first() {
    let (mut d, mut h, outer, _, _) = grouped();
    let inner = run(&mut d, &mut h, &[shape("GROUP", outer, 0.0, 0.0, 1.0, 1.0)])[0];
    let leaf = run(
        &mut d,
        &mut h,
        &[shape("RECTANGLE", inner, 0.0, 0.0, 10.0, 10.0)],
    )[0];
    run(
        &mut d,
        &mut h,
        &[EditOp::Move {
            target: leaf,
            x: 0.0,
            y: 200.0,
            relative: true,
        }],
    );
    let page = d.pages[0];
    let leaf_b = layer_bounds(&d, leaf, page).unwrap();
    let inner_b = layer_bounds(&d, inner, page).unwrap();
    let outer_b = layer_bounds(&d, outer, page).unwrap();
    assert_near((inner_b.x0, inner_b.y1), (leaf_b.x0, leaf_b.y1));
    assert_near(
        (outer_b.x1.max(leaf_b.x1), outer_b.y1),
        (outer_b.x1, leaf_b.y1),
    );
}

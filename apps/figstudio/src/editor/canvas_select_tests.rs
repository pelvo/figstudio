//! Selection gesture math: the box resize, snapping, distances, and several layers resized
//! or rotated as one.

use figstudio_fig::edit::{EditOp, History, execute_edit_ops};
use figstudio_render::kurbo::{Point, Rect as KRect};

use super::*;

fn page_with(rects: &[(f64, f64, f64, f64)]) -> (Document, History, Vec<Guid>) {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let mut doc = Document::open(path).unwrap();
    let mut history = History::new(20);
    let page = doc.pages[0];
    let ops: Vec<EditOp> = rects
        .iter()
        .map(|&(x, y, w, h)| EditOp::CreateShape {
            id: None,
            kind: "RECTANGLE".into(),
            parent: page,
            name: None,
            x,
            y,
            width: w,
            height: h,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        })
        .collect();
    let ids = execute_edit_ops(&mut doc, &mut history, &ops, None, None, false)
        .unwrap()
        .0
        .created_ids;
    (doc, history, ids)
}

fn bounds(doc: &Document, id: Guid) -> KRect {
    figstudio_render::node_bounds(doc, id).unwrap()
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

#[test]
fn box_resize_keeps_the_opposite_side_and_the_aspect_with_shift() {
    let b = KRect::new(0.0, 0.0, 100.0, 50.0);
    assert_eq!(
        box_resized(b, Handle::SE, 20.0, 10.0, false),
        KRect::new(0.0, 0.0, 120.0, 60.0)
    );
    assert_eq!(
        box_resized(b, Handle::NW, 20.0, 10.0, false),
        KRect::new(20.0, 10.0, 100.0, 50.0)
    );
    // Shift: the larger relative change wins (width +50 %), height follows.
    assert_eq!(
        box_resized(b, Handle::SE, 50.0, 5.0, true),
        KRect::new(0.0, 0.0, 150.0, 75.0)
    );
    // An edge handle with Shift scales the other side too.
    assert_eq!(
        box_resized(b, Handle::W, -100.0, 0.0, true),
        KRect::new(-100.0, 0.0, 100.0, 100.0)
    );
    // Never below one unit, never flipped.
    assert_eq!(box_resized(b, Handle::E, -500.0, 0.0, false).width(), 1.0);
}

#[test]
fn snapping_meets_the_nearest_edge_or_centre_and_draws_its_guide() {
    let other = KRect::new(100.0, 0.0, 200.0, 50.0);
    // Left edge 3 away from the other's right edge, within 5.
    let (sx, sy, guides) = snap(KRect::new(203.0, 100.0, 243.0, 140.0), &[other], 5.0);
    assert_eq!((sx, sy), (-3.0, 0.0));
    assert!(guides.contains(&Guide {
        vertical: true,
        at: 200.0,
        from: 0.0,
        to: 140.0
    }));
    // Centres: 148 → 150.
    let (sx, _, _) = snap(KRect::new(128.0, 100.0, 168.0, 140.0), &[other], 5.0);
    assert_eq!(sx, 2.0);
    // Out of reach: no shift, no guide.
    let (sx, sy, guides) = snap(KRect::new(260.0, 300.0, 270.0, 310.0), &[other], 5.0);
    assert_eq!((sx, sy), (0.0, 0.0));
    assert!(guides.is_empty());
}

#[test]
fn gaps_measure_to_the_nearest_neighbour_on_each_side() {
    let moving = KRect::new(100.0, 100.0, 150.0, 150.0);
    let right_far = KRect::new(300.0, 100.0, 320.0, 150.0);
    let right = KRect::new(170.0, 120.0, 190.0, 200.0);
    let above = KRect::new(100.0, 0.0, 150.0, 90.0);
    let elsewhere = KRect::new(400.0, 400.0, 410.0, 410.0);
    let g = gaps(moving, &[right_far, right, above, elsewhere]);
    assert!(g.contains(&(Point::new(150.0, 135.0), Point::new(170.0, 135.0))));
    assert!(g.contains(&(Point::new(125.0, 90.0), Point::new(125.0, 100.0))));
    assert_eq!(g.len(), 2);
}

#[test]
fn several_layers_resize_inside_their_shared_box() {
    let (mut doc, mut history, ids) =
        page_with(&[(0.0, 0.0, 50.0, 50.0), (100.0, 0.0, 100.0, 100.0)]);
    let layers = starts(&doc, &ids);
    // The box is 200 × 100; dragging the right edge by 200 doubles the width.
    let ops = resize_ops(&doc, &layers, Handle::E, 200.0, 0.0, false);
    execute_edit_ops(&mut doc, &mut history, &ops, None, None, false).unwrap();
    assert_eq!(bounds(&doc, ids[0]), KRect::new(0.0, 0.0, 100.0, 50.0));
    assert_eq!(bounds(&doc, ids[1]), KRect::new(200.0, 0.0, 400.0, 100.0));
}

#[test]
fn several_layers_rotate_about_their_shared_centre() {
    let (mut doc, mut history, ids) = page_with(&[(0.0, 0.0, 20.0, 20.0), (80.0, 0.0, 20.0, 20.0)]);
    let layers = starts(&doc, &ids);
    // Shared centre (50, 10); a quarter turn clockwise puts the left one above, the right
    // one below.
    let ops = rotate_ops(&doc, &layers, Point::new(50.0, 10.0), 90.0);
    execute_edit_ops(&mut doc, &mut history, &ops, None, None, false).unwrap();
    let c = |id| bounds(&doc, id).center();
    near(c(ids[0]).x, 50.0);
    near(c(ids[0]).y, -30.0);
    near(c(ids[1]).x, 50.0);
    near(c(ids[1]).y, 50.0);
    near(
        super::super::commands::current_rotation_of(&doc, ids[0]),
        90.0,
    );
}

#[test]
fn the_selection_box_of_one_rotated_layer_is_its_own_turned_box() {
    let (mut doc, mut history, ids) = page_with(&[(0.0, 0.0, 100.0, 20.0)]);
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::Rotate {
            target: ids[0],
            degrees: 45.0,
            pivot: Pivot::Center,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let mut tab = crate::tab::Tab::new(
        1,
        "t.fig".into(),
        std::sync::Arc::new(doc),
        None,
        std::time::Duration::ZERO,
    );
    tab.view.zoom = 1.0;
    tab.view.pan = egui::Vec2::ZERO;
    let q = quad(&tab, Pos2::ZERO, &ids).unwrap();
    // The top edge runs down-right at 45°, not along the page's x axis.
    let top = q[1] - q[0];
    assert!((top.x - top.y).abs() < 0.01 && top.x > 0.0, "{top:?}");
}

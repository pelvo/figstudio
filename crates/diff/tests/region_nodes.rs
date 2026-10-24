mod support;

use figstudio_diff::{RenderOpts, attach_render_diffs, diff_documents};
use figstudio_fig::{Guid, Node, Val};
use support::{document, filled_rectangle_at, object};

const RED: (f64, f64, f64) = (1.0, 0.0, 0.0);
const GREEN: (f64, f64, f64) = (0.0, 1.0, 0.0);

#[test]
fn region_membership_requires_half_the_clamped_canvas_box() {
    let side = |changed| {
        let mut nodes = vec![
            filled_rectangle_at(9, 1, RED, 399.0, 399.0, 1.0, 1.0),
            filled_rectangle_at(10, 1, RED, 0.0, 0.0, 1.0, 1.0),
            filled_rectangle_at(
                11,
                1,
                if changed { GREEN } else { RED },
                10.0,
                100.0,
                10.0,
                10.0,
            ),
        ];
        for (id, x, w, h) in [
            (2, 0.0, 400.0, 400.0),   // Covers the area but is mostly outside it.
            (3, 40.0, 8.0, 10.0),     // Exactly half inside.
            (4, 40.1, 8.0, 10.0),     // Just under half.
            (5, 39.9, 8.0, 10.0),     // Just over half.
            (6, -100.0, 160.0, 10.0), // Majority after clipping to the canvas.
            (7, -100.0, 10.0, 10.0),  // Entirely off canvas.
            (8, if changed { 300.0 } else { 40.0 }, 8.0, 10.0),
        ] {
            let mut node = filled_rectangle_at(id, 1, RED, x, 100.0, w, h);
            node.data.set("visible", Val::Bool(false));
            node.data
                .set("fontSize", Val::Float(if changed { 14.0 } else { 12.0 }));
            nodes.push(Node::from_data(node.data).unwrap());
        }
        for n in &mut nodes {
            n.data.set(
                "parentIndex",
                object(&[
                    ("guid", Guid(0, 100).to_val()),
                    ("position", Val::Str("a".into())),
                ]),
            );
            *n = Node::from_data(n.data.clone()).unwrap();
        }
        let mut frame = support::node(100, Some(1), "FRAME", "");
        frame.data.set(
            "size",
            object(&[("x", Val::Float(400.0)), ("y", Val::Float(400.0))]),
        );
        nodes.insert(0, Node::from_data(frame.data).unwrap());
        document(nodes)
    };
    let (old, new) = (side(false), side(true));
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let render = &report.pages[0].screens[0];
    assert_eq!(render.regions.len(), 1);
    assert_eq!(render.regions[0].x, 0);
    assert_eq!(render.regions[0].width, 44);
    assert_eq!(
        render.regions[0].nodes,
        vec![Guid(0, 3), Guid(0, 5), Guid(0, 6), Guid(0, 8), Guid(0, 11)]
    );
    let html = figstudio_diff::render_html(&report);
    let rest = html.split("Other changes in this screen").nth(1).unwrap();
    assert!(rest.contains("layer-2") && rest.contains("layer-4"));
    assert!(!rest.contains("layer-3") && !rest.contains("layer-6"));
}

#[test]
fn regions_include_majority_covered_layers_on_both_sides_in_report_order() {
    // Nested transform, negative page origin, non-unit render scale, and distant ink.
    let mut parent = filled_rectangle_at(2, 1, RED, -100.0, -80.0, 400.0, 200.0);
    parent.data.set("type", Val::Str("FRAME".into()));
    parent.data.set(
        "transform",
        object(&[
            ("m00", Val::Float(0.0)),
            ("m01", Val::Float(-1.0)),
            ("m10", Val::Float(1.0)),
            ("m11", Val::Float(0.0)),
            ("m02", Val::Float(-100.0)),
            ("m12", Val::Float(-80.0)),
        ]),
    );
    let parent = Node::from_data(parent.data).unwrap();
    let child = |id, color, x| filled_rectangle_at(id, 2, color, x, 20.0, 20.0, 20.0);
    let far = filled_rectangle_at(9, 1, RED, 1000.0, 0.0, 20.0, 20.0);
    let old = document(vec![
        parent.clone(),
        child(3, RED, 30.0),
        child(4, RED, 60.0),
        far.clone(),
    ]);
    let new = document(vec![
        parent,
        child(3, GREEN, 30.0),
        child(5, GREEN, 60.0),
        far,
    ]);
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(
        &old,
        &new,
        &mut report,
        &RenderOpts {
            scale: 0.5,
            ..Default::default()
        },
    );
    let regions = &report.pages[0].screens[0].regions;
    assert_eq!(regions.len(), 1);
    let crop = image::load_from_memory(&images[&Guid(0, 1)].screens[0].regions[0].after).unwrap();
    assert_eq!(
        (crop.width(), crop.height()),
        (
            regions[0].image_width.unwrap(),
            regions[0].image_height.unwrap()
        )
    );
    assert!(
        crop.width() >= 640 && crop.height() >= 320,
        "320x160 screen px of context at 2x"
    );
    let green = crop
        .to_rgba8()
        .pixels()
        .filter(|p| p.0 == [0, 255, 0, 255])
        .count();
    assert!(
        green >= 3000,
        "both 20×20 design rectangles stay sharp at device scale 2: {green}"
    );
    assert_eq!(regions[0].nodes, vec![Guid(0, 5), Guid(0, 4), Guid(0, 3)]);
    assert!(!regions[0].nodes.contains(&Guid(0, 9)));
}

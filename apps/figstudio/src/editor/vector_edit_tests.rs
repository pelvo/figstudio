//! The Pen tool's network and finished layer, and the vector edit keys.

use std::sync::Arc;
use std::time::Duration;

use egui::{Event, Modifiers};
use figstudio_fig::Document;

use super::*;

fn tab() -> Tab {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    Tab::new(
        1,
        "t.fig".into(),
        Arc::new(Document::open(path).unwrap()),
        None,
        Duration::ZERO,
    )
}

fn network_of(tab: &Tab, id: Guid) -> VectorNetwork {
    figstudio_fig::edit::outline::network(&tab.doc, &tab.doc.node(id).unwrap().data)
        .unwrap()
        .0
}

/// A triangle drawn with the pen (the first point dragged into a curve), closed.
fn triangle(tab: &mut Tab) -> Guid {
    tab.pen = Some(PenPath {
        points: vec![
            (pos2(100.0, 100.0), vec2(10.0, 0.0)),
            (pos2(200.0, 100.0), Vec2::ZERO),
            (pos2(150.0, 180.0), Vec2::ZERO),
        ],
        dragging: false,
    });
    finish_pen(tab, true);
    tab.selection.expect("the new vector is selected")
}

#[test]
fn the_pen_makes_a_stroked_vector_with_mirrored_handles() {
    let net = pen_network(
        &[
            (pos2(10.0, 10.0), vec2(5.0, 0.0)),
            (pos2(30.0, 10.0), vec2(2.0, 5.0)),
        ],
        false,
        pos2(10.0, 10.0),
    );
    assert_eq!(net.vertices.len(), 2);
    assert_eq!((net.vertices[1].x, net.vertices[1].y), (20.0, 0.0));
    let s = net.segments[0];
    assert_eq!(s.tangent_start, (5.0, 0.0));
    assert_eq!(
        s.tangent_end,
        (-2.0, -5.0),
        "the incoming handle mirrors the outgoing one"
    );
    assert!(net.regions.is_empty(), "an open path has no region");

    let mut tab = tab();
    let id = triangle(&mut tab);
    let n = tab.doc.node(id).unwrap();
    assert_eq!(n.kind, "VECTOR");
    assert!(
        n.data
            .get("fillPaints")
            .is_none_or(|p| p.items().is_empty())
    );
    assert_eq!(n.data.get("strokePaints").map(|p| p.items().len()), Some(1));
    let net = network_of(&tab, id);
    assert_eq!(
        (net.vertices.len(), net.segments.len(), net.regions.len()),
        (3, 3, 1)
    );
    let b = figstudio_render::node_bounds(&tab.doc, id).unwrap();
    assert_eq!((b.x0, b.y0), (100.0, 100.0));
    assert!(tab.reset_tool_to_select, "the toolbar goes back to Select");
    // One undo removes the whole path.
    crate::editor::undo_tab(&mut tab).unwrap();
    assert!(tab.doc.node(id).is_none());
}

fn press(tab: &mut Tab, key: Key, modifiers: Modifiers) {
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput {
        events: vec![
            Event::ModifiersChanged(modifiers),
            Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
        ],
        ..Default::default()
    });
    keys(&ctx, tab, false);
    let _ = ctx.end_pass();
}

#[test]
fn enter_edits_points_arrows_nudge_delete_removes_escape_leaves() {
    let mut tab = tab();
    let id = triangle(&mut tab);
    press(&mut tab, Key::Enter, Modifiers::NONE);
    assert!(tab.vector_edit.as_ref().is_some_and(|v| v.node == id));
    tab.vector_edit.as_mut().unwrap().selected = Some(1);
    press(&mut tab, Key::ArrowRight, Modifiers::SHIFT);
    // Relative to the first point: the box refits to the curves, which can move every point.
    let net = network_of(&tab, id);
    let (a, v) = (net.vertices[0], net.vertices[1]);
    assert!(
        (v.x - a.x - 110.0).abs() < 1e-3 && (v.y - a.y).abs() < 1e-3,
        "{a:?} {v:?}"
    );
    press(&mut tab, Key::Delete, Modifiers::NONE);
    assert_eq!(network_of(&tab, id).vertices.len(), 2);
    press(&mut tab, Key::Escape, Modifiers::NONE);
    assert!(tab.vector_edit.is_none());
    assert_eq!(tab.selection, Some(id), "the vector stays selected");
}

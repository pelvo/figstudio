//! Focused headless egui pointer regressions for overlapping bent arrows, grouped lines,
//! gizmo empty-interior pass-through, and outer-page hover caching.
use super::pick::{FRAME, GROUP, NEST, TEXT, object as obj};
use super::*;
use egui::{Modifiers, Pos2};
use figstudio_fig::{Document, Guid, Node, Val};
use figstudio_render::kurbo::Point;
use std::sync::Arc;

const ARROW: Guid = Guid(99, 50);
const ARROW_H: Guid = Guid(99, 51);
const ARROW_V: Guid = Guid(99, 52);

fn solid(r: f64, g: f64, b: f64) -> Val {
    Val::Arr(vec![obj(&[
        ("type", Val::Str("SOLID".into())),
        (
            "color",
            obj(&[
                ("r", Val::Float(r)),
                ("g", Val::Float(g)),
                ("b", Val::Float(b)),
                ("a", Val::Float(1.0)),
            ]),
        ),
    ])])
}

fn size_val(w: f64, h: f64) -> Val {
    obj(&[("x", Val::Float(w)), ("y", Val::Float(h))])
}

fn transform_val(x: f64, y: f64) -> Val {
    obj(&[
        ("m00", Val::Float(1.0)),
        ("m01", Val::Float(0.0)),
        ("m02", Val::Float(x)),
        ("m10", Val::Float(0.0)),
        ("m11", Val::Float(1.0)),
        ("m12", Val::Float(y)),
    ])
}

fn poly_blob(points: &[(f32, f32)]) -> Vec<u8> {
    let mut b = Vec::new();
    for (i, &(x, y)) in points.iter().enumerate() {
        b.push(if i == 0 { 1 } else { 2 });
        b.extend(x.to_le_bytes());
        b.extend(y.to_le_bytes());
    }
    b.push(0);
    b
}

/// L-shaped 2px bent arrow along top (y=0..2, x=0..220) and right (x=218..220, y=0..160) edges
/// of a 220x160 box; interior around `pick::TEXT` is empty off-stroke space.
fn bent_arrow_geom(doc: &mut Document) -> Val {
    let top = doc.blobs.len() as i64;
    doc.blobs.push(poly_blob(&[
        (0.0, 0.0),
        (220.0, 0.0),
        (220.0, 2.0),
        (0.0, 2.0),
    ]));
    let right = doc.blobs.len() as i64;
    doc.blobs.push(poly_blob(&[
        (218.0, 0.0),
        (220.0, 0.0),
        (220.0, 160.0),
        (218.0, 160.0),
    ]));
    Val::Arr(vec![
        obj(&[("commandsBlob", Val::Int(top))]),
        obj(&[("commandsBlob", Val::Int(right))]),
    ])
}

fn base_node(
    id: Guid,
    parent: Guid,
    position: &str,
    kind: &str,
    (w, h): (f64, f64),
    (x, y): (f64, f64),
) -> Node {
    Node::from_data(obj(&[
        ("guid", id.to_val()),
        ("type", Val::Str(kind.into())),
        (
            "parentIndex",
            obj(&[
                ("guid", parent.to_val()),
                ("position", Val::Str(position.into())),
            ]),
        ),
        ("size", size_val(w, h)),
        ("transform", transform_val(x, y)),
        ("frameMaskDisabled", Val::Bool(true)),
    ]))
    .unwrap()
}

fn harness_with(doc: Document) -> Harness<'static, App> {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    h.state_mut().reduce_motion = true;
    let tab = &mut h.state_mut().tabs[0];
    tab.doc = Arc::new(doc);
    tab.set_page(Guid(99, 1));
    tab.fit_pending = true;
    h.run_steps(4);
    h
}

fn page_to_screen(h: &Harness<'_, App>, x: f32, y: f32) -> Pos2 {
    let tab = &h.state().tabs[0];
    tab.view
        .to_screen(tab.canvas_rect.unwrap().min, Pos2::new(x, y))
}

fn drag_pointer(h: &mut Harness<'_, App>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.step();
    press(h, from, PointerButton::Primary, true);
    h.step();
    for i in 1..=3 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 3.0)));
        h.step();
    }
    press(h, to, PointerButton::Primary, false);
    h.step();
}

fn doc_with_bent_vector(parent: Guid) -> Document {
    let mut doc = pick::nested();
    let geom = bent_arrow_geom(&mut doc);
    let mut v = base_node(
        ARROW,
        parent,
        "999",
        "VECTOR",
        (220.0, 160.0),
        (-10.0, -10.0),
    );
    v.data.set("fillPaints", Val::Arr(vec![]));
    v.data.set("strokePaints", solid(0.0, 0.0, 0.0));
    v.data.set("strokeWeight", Val::Float(2.0));
    v.data.set("strokeGeometry", geom);
    let mut nodes = doc.nodes.clone();
    nodes.push(v);
    doc.set_nodes(nodes).unwrap();
    doc
}

fn assert_off_stroke_double_click_descends_into_text(h: &mut Harness<'_, App>) {
    let at = centre_of(h, TEXT);
    primary_clicks(h, at, egui::Vec2::ZERO, 1);
    assert_eq!(h.state().tabs[0].selection, Some(GROUP));

    h.run_steps(24);
    primary_clicks(h, at, egui::Vec2::ZERO, 2);
    assert_eq!(h.state().tabs[0].selection, Some(NEST));
    assert!(h.state().tabs[0].text_select.is_none());

    h.run_steps(24);
    primary_clicks(h, at, egui::Vec2::ZERO, 2);
    assert_eq!(h.state().tabs[0].selection, Some(TEXT));
    assert!(h.state().tabs[0].text_select.is_some());
}

#[test]
fn bent_vector_pointer_flow_descends_off_stroke_drags_on_stroke_and_ignores_empty_interior_gizmo() {
    for parent in [Guid(99, 1), FRAME] {
        let mut h = harness_with(doc_with_bent_vector(parent));
        assert_off_stroke_double_click_descends_into_text(&mut h);

        // Cmd+click off-stroke deep-selects TEXT directly.
        h.state_mut().tabs[0].select(None);
        let at = centre_of(&h, TEXT);
        pick::click(&mut h, at, Modifiers::COMMAND);
        assert_eq!(h.state().tabs[0].selection, Some(TEXT));

        // Direct single-click on visible stroke selects ARROW.
        h.run_steps(60);
        let on_stroke = page_to_screen(&h, 40.0, -9.0);
        primary_clicks(&mut h, on_stroke, egui::Vec2::ZERO, 1);
        assert_eq!(h.state().tabs[0].selection, Some(ARROW));

        // Dragging in ARROW's empty off-stroke interior while ARROW is selected must NOT move it.
        let before_empty_drag =
            figstudio_render::absolute_transform(&h.state().tabs[0].doc, ARROW) * Point::ZERO;
        drag_pointer(&mut h, at, at + egui::vec2(24.0, 12.0));
        let after_empty_drag =
            figstudio_render::absolute_transform(&h.state().tabs[0].doc, ARROW) * Point::ZERO;
        assert!(
            (after_empty_drag - before_empty_drag).hypot() < 1e-3,
            "parent {parent}: dragging empty interior of selected arrow must not move it"
        );
        assert!(
            !h.state().tabs[0].dirty_unsaved,
            "parent {parent}: empty interior drag must not dirty the document"
        );

        // Dragging directly on the visible stroke moves ARROW cleanly.
        h.state_mut().tabs[0].select(Some(ARROW));
        h.run_steps(4);
        drag_pointer(&mut h, on_stroke, on_stroke + egui::vec2(24.0, 0.0));
        let after_stroke_drag =
            figstudio_render::absolute_transform(&h.state().tabs[0].doc, ARROW) * Point::ZERO;
        assert!(
            after_stroke_drag.x > before_empty_drag.x + 5.0,
            "parent {parent}: dragging on visible stroke must move ARROW"
        );

        // Clicking empty canvas outside FRAME deselects.
        h.run_steps(60);
        let empty = page_to_screen(&h, -60.0, -60.0);
        primary_clicks(&mut h, empty, egui::Vec2::ZERO, 1);
        assert_eq!(h.state().tabs[0].selection, None);
    }
}

#[test]
fn grouped_bent_lines_pass_off_stroke_and_select_group_then_line() {
    for figma_group in [false, true] {
        let mut doc = pick::nested();
        let kind = if figma_group { "FRAME" } else { "GROUP" };
        let mut grp = base_node(
            ARROW,
            Guid(99, 1),
            "999",
            kind,
            (220.0, 160.0),
            (-10.0, -10.0),
        );
        if figma_group {
            grp.data.set("resizeToFit", Val::Bool(true));
        }
        let mut horiz = base_node(ARROW_H, ARROW, "001", "LINE", (220.0, 0.0), (0.0, 1.0));
        horiz.data.set("strokePaints", solid(0.0, 0.0, 0.0));
        horiz.data.set("strokeWeight", Val::Float(2.0));
        let mut vert = base_node(ARROW_V, ARROW, "002", "LINE", (160.0, 0.0), (219.0, 0.0));
        vert.data.set(
            "transform",
            obj(&[
                ("m00", Val::Float(0.0)),
                ("m01", Val::Float(-1.0)),
                ("m10", Val::Float(1.0)),
                ("m11", Val::Float(0.0)),
                ("m02", Val::Float(219.0)),
                ("m12", Val::Float(0.0)),
            ]),
        );
        vert.data.set("strokePaints", solid(0.0, 0.0, 0.0));
        vert.data.set("strokeWeight", Val::Float(2.0));
        let mut nodes = doc.nodes.clone();
        nodes.extend([grp, horiz, vert]);
        doc.set_nodes(nodes).unwrap();

        let mut h = harness_with(doc);
        assert_off_stroke_double_click_descends_into_text(&mut h);

        h.run_steps(60);
        let on_line = page_to_screen(&h, 100.0, -9.0);
        primary_clicks(&mut h, on_line, egui::Vec2::ZERO, 1);
        assert_eq!(h.state().tabs[0].selection, Some(ARROW));
        h.run_steps(24);
        primary_clicks(&mut h, on_line, egui::Vec2::ZERO, 2);
        assert_eq!(h.state().tabs[0].selection, Some(ARROW_H));
    }
}

#[test]
fn stationary_outer_hover_reuses_page_hit_cache_and_invalidates_on_edit() {
    let mut h = harness_with(doc_with_bent_vector(Guid(99, 1)));
    let at = centre_of(&h, TEXT);
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    let warmed = h.state().tabs[0]
        .pick_cache
        .page_walks(&h.state().tabs[0].doc);
    assert!(warmed >= 1);

    // Repeated stationary hover frames must do zero additional outer-page hit walks.
    for _ in 0..8 {
        h.run_steps(1);
    }
    assert_eq!(
        h.state().tabs[0]
            .pick_cache
            .page_walks(&h.state().tabs[0].doc),
        warmed,
        "stationary outer-page hover must reuse PickCache::page_chain"
    );
    assert_eq!(h.state().tabs[0].hover, Some(GROUP));

    // Moving the pointer to the visible stroke triggers one fresh walk.
    let on_stroke = page_to_screen(&h, 40.0, -9.0);
    h.event(Event::PointerMoved(on_stroke));
    h.run_steps(1);
    assert_eq!(
        h.state().tabs[0]
            .pick_cache
            .page_walks(&h.state().tabs[0].doc),
        warmed + 1
    );
    assert_eq!(h.state().tabs[0].hover, Some(ARROW));

    // Mutating the document assigns a new doc.instance and invalidates the cache.
    let mut updated = (*h.state().tabs[0].doc).clone();
    let mut nodes = updated.nodes.clone();
    nodes
        .iter_mut()
        .find(|n| n.id == ARROW)
        .unwrap()
        .data
        .set("visible", Val::Bool(false));
    updated.set_nodes(nodes).unwrap();
    h.state_mut().tabs[0].doc = Arc::new(updated);
    h.run_steps(1);
    assert_eq!(
        h.state().tabs[0]
            .pick_cache
            .page_walks(&h.state().tabs[0].doc),
        1
    );
    assert_ne!(h.state().tabs[0].hover, Some(ARROW));
}

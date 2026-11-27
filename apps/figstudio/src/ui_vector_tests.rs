//! Paths on the real app: the Pen tool draws one, a double-click edits its points.
use egui::{Event, Key, Modifiers, PointerButton};

use super::*;

fn screen(h: &Harness<'_, App>, x: f32, y: f32) -> Pos2 {
    let tab = &h.state().tabs[0];
    tab.view
        .to_screen(tab.canvas_rect.expect("laid out").min, Pos2::new(x, y))
}

fn button(h: &mut Harness<'_, App>, at: Pos2, pressed: bool) {
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(1);
}

fn click(h: &mut Harness<'_, App>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    button(h, at, true);
    button(h, at, false);
}

fn network(h: &Harness<'_, App>, id: Guid) -> figstudio_fig::edit::vector_net::VectorNetwork {
    let tab = &h.state().tabs[0];
    figstudio_fig::edit::outline::network(&tab.doc, &tab.doc.node(id).unwrap().data)
        .unwrap()
        .0
}

/// Draws a closed triangle with the Pen tool (P) on an empty page; returns the vector.
fn draw_triangle(h: &mut Harness<'_, App>) -> Guid {
    crate::editor::apply_tab_ops(
        &mut h.state_mut().tabs[0],
        &[figstudio_fig::edit::EditOp::CreatePage {
            id: None,
            name: "Paths".into(),
        }],
        None,
        None,
    )
    .map(|ids| h.state_mut().tabs[0].set_page(ids[0]))
    .unwrap();
    h.run_steps(3);
    h.key_press(Key::P);
    h.run_steps(1);
    assert_eq!(h.state().tool, crate::canvas::Tool::Pen);
    for (x, y) in [
        (100.0, 100.0),
        (200.0, 100.0),
        (150.0, 180.0),
        (100.0, 100.0),
    ] {
        let at = screen(h, x, y);
        click(h, at);
    }
    h.run_steps(2);
    let tab = &h.state().tabs[0];
    let id = tab.selection.expect("the path is selected");
    assert_eq!(tab.doc.node(id).unwrap().kind, "VECTOR");
    id
}

#[test]
fn the_pen_draws_a_closed_path_and_returns_to_select() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    let id = draw_triangle(&mut h);
    let net = network(&h, id);
    assert_eq!(
        (net.vertices.len(), net.segments.len(), net.regions.len()),
        (3, 3, 1)
    );
    assert_eq!(h.state().tool, crate::canvas::Tool::Select);
    assert!(h.state().tabs[0].pen.is_none());
}

#[test]
fn double_click_edits_points_and_a_click_away_leaves() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    let id = draw_triangle(&mut h);
    h.run_steps(30);
    let inside = screen(&h, 150.0, 120.0);
    super::text_select::double_click(&mut h, &mut Vec::new(), inside);
    h.run_steps(2);
    assert!(h.state().tabs[0].vector_edit.is_some(), "edit mode");
    // Drag the right point (200, 100) to (240, 90).
    let (from, to) = (screen(&h, 200.0, 100.0), screen(&h, 240.0, 90.0));
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    button(&mut h, from, true);
    for i in 1..=4 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 4.0)));
        h.run_steps(1);
    }
    button(&mut h, to, false);
    let v = network(&h, id).vertices[1];
    // In layer units: the layer box starts at (100, 100), then grows to fit the point.
    let b = figstudio_render::node_bounds(&h.state().tabs[0].doc, id).unwrap();
    assert!(
        (b.x1 - 240.0).abs() < 0.6 && (b.y0 - 90.0).abs() < 0.6,
        "{b:?} {v:?}"
    );
    // A click far away leaves edit mode.
    let away = screen(&h, 600.0, 600.0);
    click(&mut h, away);
    assert!(h.state().tabs[0].vector_edit.is_none());
}

fn drag(h: &mut Harness<'_, App>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    button(h, from, true);
    for i in 1..=4 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 4.0)));
        h.run_steps(1);
    }
    button(h, to, false);
}

#[test]
fn a_selected_point_drags_again_and_a_curve_handle_bends_its_segment() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    // A path whose first point was dragged into a curve: its handle sticks out 30 to the right.
    let tab = &mut h.state_mut().tabs[0];
    tab.pen = Some(crate::editor::PenPath {
        points: vec![
            (Pos2::new(100.0, 100.0), egui::vec2(30.0, 0.0)),
            (Pos2::new(200.0, 100.0), egui::Vec2::ZERO),
            (Pos2::new(150.0, 180.0), egui::Vec2::ZERO),
        ],
        ..Default::default()
    });
    crate::editor::vector_edit_finish_pen(tab, false);
    let id = tab.selection.unwrap();
    h.run_steps(3);
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert!(h.state().tabs[0].vector_edit.is_some());
    // Clicking a point selects it and stays in edit mode, also just outside the shape (where
    // picking layers would find none).
    let p1 = screen(&h, 200.0, 100.0);
    click(&mut h, p1 + egui::vec2(4.0, -4.0));
    assert_eq!(
        h.state().tabs[0].vector_edit.as_ref().unwrap().selected,
        Some(1)
    );
    // A selected straight point drags again (its handles sit on it and are not picked).
    let to = screen(&h, 210.0, 100.0);
    drag(&mut h, p1, to);
    let net = network(&h, id);
    assert!(
        (net.vertices[1].x - net.vertices[0].x - 110.0).abs() < 0.6,
        "{net:?}"
    );
    // Select the first point, then drag its handle down.
    let p0 = screen(&h, 100.0, 100.0);
    click(&mut h, p0);
    let (from, to) = (screen(&h, 130.0, 100.0), screen(&h, 130.0, 130.0));
    drag(&mut h, from, to);
    let t = network(&h, id).segments[0].tangent_start;
    assert!(
        (t.0 - 30.0).abs() < 0.6 && (t.1 - 30.0).abs() < 0.6,
        "{t:?}"
    );
}

#[test]
fn escape_finishes_an_open_pen_path() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    h.key_press(Key::P);
    h.run_steps(1);
    for (x, y) in [(100.0, 100.0), (200.0, 150.0)] {
        let at = screen(&h, x, y);
        click(&mut h, at);
    }
    h.key_press(Key::Escape);
    h.run_steps(2);
    let tab = &h.state().tabs[0];
    let id = tab.selection.expect("the open path is selected");
    let net = network(&h, id);
    assert_eq!(
        (net.vertices.len(), net.segments.len(), net.regions.len()),
        (2, 1, 0)
    );
    assert_eq!(h.state().tool, crate::canvas::Tool::Select);
}

#[test]
fn closing_a_path_with_a_real_click_keeps_the_new_path_selected() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    crate::editor::apply_tab_ops(
        &mut h.state_mut().tabs[0],
        &[figstudio_fig::edit::EditOp::CreatePage {
            id: None,
            name: "Paths".into(),
        }],
        None,
        None,
    )
    .map(|ids| h.state_mut().tabs[0].set_page(ids[0]))
    .unwrap();
    // A frame under the path: a stray pick would select it.
    let page = h.state().tabs[0].page;
    crate::editor::apply_tab_ops(
        &mut h.state_mut().tabs[0],
        &[figstudio_fig::edit::EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: None,
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 400.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }],
        None,
        None,
    )
    .unwrap();
    h.run_steps(3);
    h.key_press(Key::P);
    h.run_steps(1);
    // Press and release in separate frames, as a person clicks; the closing click lands just
    // outside the shape (within the first point's reach), over the frame.
    for (x, y, off) in [
        (100.0, 100.0, 0.0),
        (200.0, 100.0, 0.0),
        (150.0, 180.0, 0.0),
        (100.0, 100.0, -4.0),
    ] {
        let at = screen(&h, x, y) + egui::vec2(off, off);
        h.event(Event::PointerMoved(at));
        h.run_steps(1);
        button(&mut h, at, true);
        h.run_steps(2);
        button(&mut h, at, false);
        h.run_steps(1);
    }
    let tab = &h.state().tabs[0];
    let sel = tab.selection.expect("selected");
    assert_eq!(
        tab.doc.node(sel).unwrap().kind,
        "VECTOR",
        "the new path, not the frame"
    );
}

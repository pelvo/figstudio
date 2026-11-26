//! Multi-selection on the real canvas: Shift-click, box select, Select all, and actions that
//! take the whole selection.
#![expect(non_snake_case, reason = "A and B name the two rectangles")]
use egui::{Event, Modifiers, PointerButton, vec2};

use super::*;

/// A new page holding rectangles `A` and `B` side by side and a frame with a child; the view
/// fitted to it. Returns `(a, b, frame)`.
fn layout(h: &mut Harness<'_, App>) -> (Guid, Guid, Guid) {
    use figstudio_fig::edit::EditOp;
    let shape = |kind: &str, parent: Guid, x: f64| EditOp::CreateShape {
        id: None,
        kind: kind.into(),
        parent,
        name: None,
        x,
        y: 0.0,
        width: 60.0,
        height: 40.0,
        corner_radius: 0.0,
        fill_hex: Some("#3366FF".into()),
        count: None,
        star_inner_scale: None,
    };
    let tab = &mut h.state_mut().tabs[0];
    let page = crate::editor::apply_tab_ops(
        tab,
        &[EditOp::CreatePage {
            id: None,
            name: "Multi".into(),
        }],
        None,
        None,
    )
    .unwrap()[0];
    let ids = crate::editor::apply_tab_ops(
        tab,
        &[
            shape("RECTANGLE", page, 0.0),
            shape("RECTANGLE", page, 100.0),
            shape("FRAME", page, 200.0),
        ],
        None,
        None,
    )
    .unwrap();
    crate::editor::apply_tab_ops(tab, &[shape("RECTANGLE", ids[2], 210.0)], None, None).unwrap();
    tab.set_page(page);
    tab.select(None);
    tab.fit_pending = true;
    for _ in 0..6 {
        h.step();
    }
    (ids[0], ids[1], ids[2])
}

fn shift_click(h: &mut Harness<'_, App>, c: &mut Vec<String>, at: Pos2) {
    // egui's held modifiers come from `ModifiersChanged`, as from a real keyboard.
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.event(Event::PointerMoved(at));
    step(h, c);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::SHIFT,
        });
    }
    step(h, c);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    step(h, c);
}

fn click(h: &mut Harness<'_, App>, c: &mut Vec<String>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    step(h, c);
    press(h, at, PointerButton::Primary, true);
    press(h, at, PointerButton::Primary, false);
    step(h, c);
}

#[test]
fn shift_click_adds_and_removes_layers_and_a_plain_click_starts_over() {
    let (mut h, mut c) = (harness(), Vec::new());
    let (a_id, b_id, _) = layout(&mut h);
    let (A, B) = (a_id, b_id);
    let (a, b) = (centre_of(&h, A), centre_of(&h, B));
    click(&mut h, &mut c, a);
    shift_click(&mut h, &mut c, b);
    assert_eq!(h.state().tabs[0].selected(), [A, B]);
    shift_click(&mut h, &mut c, a);
    assert_eq!(
        h.state().tabs[0].selected(),
        [B],
        "removing the primary promotes the next"
    );
    shift_click(&mut h, &mut c, a);
    click(&mut h, &mut c, a);
    assert_eq!(h.state().tabs[0].selected(), [A]);
}

#[test]
fn a_box_selects_the_layers_it_encloses_and_keeps_the_region_menu() {
    let (mut h, mut c) = (harness(), Vec::new());
    let (a_id, b_id, frame) = layout(&mut h);
    let (A, B) = (a_id, b_id);
    let tab = &h.state().tabs[0];
    let enclose = [A, B, frame]
        .into_iter()
        .filter_map(|id| figstudio_render::node_bounds(&tab.doc, id))
        .reduce(|x, y| x.union(y))
        .unwrap();
    let origin = tab.canvas_rect.unwrap().min;
    let to = |x: f64, y: f64| tab.view.to_screen(origin, Pos2::new(x as f32, y as f32));
    let (from, until) = (
        to(enclose.x0, enclose.y0) - vec2(8.0, 8.0),
        to(enclose.x1, enclose.y1) + vec2(8.0, 8.0),
    );
    drag(&mut h, &mut c, from, until);
    h.step();
    let sel = h.state().tabs[0].selected();
    assert_eq!(
        sel,
        [A, B, frame],
        "the enclosed top-level layers, in z order"
    );
    assert!(h.state().tabs[0].marquee.rect().is_some());
    right_click(&mut h, &mut c, from + (until - from) / 2.0);
    h.step();
    assert!(h.query_by_label("Copy region").is_some());
}

#[test]
fn delete_and_duplicate_act_on_every_selected_layer_in_one_undo_step() {
    let mut h = harness();
    let (a_id, b_id, _) = layout(&mut h);
    let (A, B) = (a_id, b_id);
    let ctx = egui::Context::default();
    h.state_mut().tabs[0].select_all_of(&[A, B]);
    let depth = h.state().tabs[0].history.undo_len();
    h.state_mut()
        .apply(crate::menu::Action::DuplicateSelection, &ctx);
    let copies = h.state().tabs[0].selected();
    assert_eq!(copies.len(), 2, "both copies are selected");
    assert!(!copies.contains(&A) && !copies.contains(&B));
    assert_eq!(h.state().tabs[0].history.undo_len(), depth + 1);

    h.state_mut()
        .apply(crate::menu::Action::DeleteSelection, &ctx);
    let doc = &h.state().tabs[0].doc;
    assert!(copies.iter().all(|id| doc.node(*id).is_none()));
    assert!(h.state().tabs[0].selected().is_empty());
    // Undo brings the copies back; the selection holds only layers that exist.
    h.state_mut().apply(crate::menu::Action::Undo, &ctx);
    let tab = &h.state().tabs[0];
    assert!(tab.selected().iter().all(|id| tab.doc.node(*id).is_some()));
}

#[test]
fn select_all_selects_the_page_layers_outside_text_editing() {
    let mut h = harness();
    let (a, b, frame) = layout(&mut h);
    let ctx = egui::Context::default();
    h.state_mut()
        .apply(crate::menu::Action::SelectAllText, &ctx);
    assert_eq!(
        h.state().tabs[0].selected(),
        [a, b, frame],
        "the top-level layers only"
    );
}

#[test]
fn command_a_on_the_focused_canvas_selects_the_page_layers() {
    let (mut h, mut c) = (harness(), Vec::new());
    let (a, b, frame) = layout(&mut h);
    let tab = &h.state().tabs[0];
    let empty = tab.canvas_rect.unwrap().right_bottom() - vec2(20.0, 20.0);
    click(&mut h, &mut c, empty);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    step(&mut h, &mut c);
    assert_eq!(h.state().tabs[0].selected(), [a, b, frame]);
}

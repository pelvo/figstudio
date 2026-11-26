//! Multi-selection on the real canvas: Shift-click, box select, Select all, and actions that
//! take the whole selection.
#![expect(non_snake_case, reason = "A and B name the two rectangles")]
use egui::{Event, Modifiers, PointerButton, vec2};

use super::*;

/// A new page holding rectangles `A` and `B` side by side and a frame with a child; the view
/// fitted to it. Returns `(a, b, frame)`.
pub(super) fn layout(h: &mut Harness<'_, App>) -> (Guid, Guid, Guid) {
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

/// Clicks the menu item whose label starts with `label` (the shortcut follows it).
fn click_menu(h: &mut Harness<'_, App>, c: &mut Vec<String>, label: &str) {
    h.get_by_label_contains(label).click();
    for _ in 0..4 {
        step(h, c);
    }
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

#[test]
fn group_shortcut_and_ungroup_round_trip_the_selection() {
    let (mut h, mut c) = (harness(), Vec::new());
    let (a, b, _) = layout(&mut h);
    h.state_mut().tabs[0].select_all_of(&[a, b]);
    let empty = h.state().tabs[0].canvas_rect.unwrap().right_bottom() - vec2(20.0, 20.0);
    h.event(Event::PointerMoved(empty));
    step(&mut h, &mut c);
    h.key_press_modifiers(Modifiers::ALT | Modifiers::COMMAND, egui::Key::G);
    step(&mut h, &mut c);
    let tab = &h.state().tabs[0];
    let group = tab.selection.expect("the group is selected");
    assert_eq!(tab.doc.children(group), &[a, b]);
    // Ungroup selects the freed layers.
    h.key_press_modifiers(
        Modifiers::SHIFT | Modifiers::ALT | Modifiers::COMMAND,
        egui::Key::G,
    );
    step(&mut h, &mut c);
    let tab = &h.state().tabs[0];
    assert!(tab.doc.node(group).is_none());
    assert_eq!(tab.selected(), [a, b]);
}

#[test]
fn the_canvas_menu_offers_edit_commands_for_the_selection_and_runs_them() {
    let (mut h, mut c) = (harness(), Vec::new());
    let (a, b, _) = layout(&mut h);
    h.state_mut().tabs[0].select_all_of(&[a, b]);
    let at = centre_of(&h, a);
    right_click(&mut h, &mut c, at);
    h.step();
    // Right-clicking a selected layer keeps the whole selection.
    assert_eq!(h.state().tabs[0].selected(), [a, b]);
    // An item's accessible label ends with its shortcut ("Group selection⌥⌘G").
    for label in ["Group selection", "Duplicate layer", "Copy name"] {
        assert!(
            h.query_by_label_contains(label).is_some(),
            "{label} is in the menu"
        );
    }
    click_menu(&mut h, &mut c, "Group selection");
    let tab = &h.state().tabs[0];
    let group = tab.selection.expect("the group is selected");
    assert_eq!(tab.doc.children(group), &[a, b]);
}

#[test]
fn a_layer_row_menu_selects_the_row_and_hides_it() {
    let (mut h, mut c) = (harness(), Vec::new());
    let (a, b, _) = layout(&mut h);
    h.state_mut().tabs[0].select(Some(a));
    h.run_steps(3);
    let row = h
        .get_by(|n| n.author_id() == Some(&format!("layers.{b}")))
        .rect()
        .center();
    right_click(&mut h, &mut c, row);
    h.step();
    assert_eq!(h.state().tabs[0].selected(), [b], "the row is selected");
    click_menu(&mut h, &mut c, "Show/Hide");
    let hidden = h.state().tabs[0].doc.node(b).unwrap().data.flag("visible");
    assert_eq!(hidden, Some(false));
}

#[test]
fn edit_commands_are_disabled_where_they_do_not_apply() {
    use crate::editor::commands::EditCmd;
    let mut h = harness();
    let (a, _, frame) = layout(&mut h);
    let tab = &mut h.state_mut().tabs[0];
    tab.select(Some(a));
    assert!(!EditCmd::Ungroup.enabled(tab));
    assert!(!EditCmd::Distribute(figstudio_fig::edit::Axis::Horizontal).enabled(tab));
    assert!(
        !EditCmd::CreateComponent.enabled(tab),
        "a rectangle is not a frame"
    );
    tab.select(Some(frame));
    assert!(EditCmd::CreateComponent.enabled(tab));
    assert!(crate::editor::commands::run(tab, EditCmd::Ungroup).is_err());
}

#[test]
fn edit_shortcuts_wait_while_a_text_field_has_focus() {
    let (mut h, mut c) = (harness(), Vec::new());
    let (a, b, _) = layout(&mut h);
    h.state_mut().tabs[0].select_all_of(&[a, b]);
    // ⌥A types "å" into a field; it must not align the selection.
    let before = h.state().tabs[0].doc.revision;
    h.get_by(|n| n.author_id() == Some("field.filter-layers"))
        .click();
    step(&mut h, &mut c);
    h.key_press_modifiers(Modifiers::ALT, egui::Key::A);
    h.key_press_modifiers(Modifiers::ALT | Modifiers::COMMAND, egui::Key::G);
    step(&mut h, &mut c);
    assert_eq!(h.state().tabs[0].doc.revision, before, "no edit ran");
}

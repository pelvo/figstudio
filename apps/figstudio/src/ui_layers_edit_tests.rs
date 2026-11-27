//! Editing from the Layers panel on the real app: Cmd and Shift selection, double-click to
//! rename, the eye and lock buttons, and dragging a row into a frame.
#![expect(non_snake_case, reason = "A, B and F name the layers")]
use egui::{Event, Key, Modifiers, PointerButton};
use egui_kittest::kittest::NodeT;

use super::multi_select::layout;
use super::*;

/// The centre of the panel control whose automation id is `author`.
fn at(h: &Harness<'_, App>, author: &str) -> Pos2 {
    let node = h
        .root()
        .children_recursive()
        .map(|n| n.accesskit_node())
        .find(|a| a.data().author_id() == Some(author))
        .unwrap_or_else(|| panic!("no control {author}"));
    let b = node.bounding_box().expect("bounds");
    Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

/// Clicks the control `author` with `modifiers` held.
fn click_on(h: &mut Harness<'_, App>, author: &str, modifiers: Modifiers) {
    let pos = at(h, author);
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(pos));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        });
    }
    h.run_steps(1);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
}

fn row(id: Guid) -> String {
    format!("layers.{id}")
}

#[test]
fn cmd_click_adds_a_row_and_shift_click_selects_the_rows_between() {
    let mut h = harness();
    let (A, B, F) = layout(&mut h);
    // Listed front first: F, B, A.
    click_on(&mut h, &row(A), Modifiers::NONE);
    click_on(&mut h, &row(F), Modifiers::COMMAND);
    let mut sel = h.state().tabs[0].selected();
    sel.sort();
    let mut want = vec![A, F];
    want.sort();
    assert_eq!(sel, want);
    click_on(&mut h, &row(A), Modifiers::NONE);
    click_on(&mut h, &row(F), Modifiers::SHIFT);
    let sel = h.state().tabs[0].selected();
    assert_eq!(sel[0], A, "the anchor stays primary");
    let mut sel = sel;
    sel.sort();
    let mut want = vec![A, B, F];
    want.sort();
    assert_eq!(sel, want, "the rows between are selected");
}

#[test]
fn double_click_renames_and_escape_keeps_the_name() {
    // Real frame times: egui's double-click window is 0.3 s.
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    let (A, _, _) = layout(&mut h);
    let name = |h: &Harness<'_, App>| h.state().tabs[0].doc.node(A).unwrap().name.clone();
    let before = name(&h);
    let double = |h: &mut Harness<'_, App>| {
        let pos = at(h, &row(A));
        super::text_select::double_click(h, &mut Vec::new(), pos);
        h.run_steps(2);
    };
    let key = |h: &mut Harness<'_, App>, key: Key| {
        h.event(Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(2);
    };
    double(&mut h);
    assert!(
        h.query_by_label("Rename layer").is_some(),
        "a double-click opens the name field"
    );
    h.event(Event::Key {
        key: Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    });
    h.event(Event::Text("Hero".into()));
    h.run_steps(1);
    key(&mut h, Key::Escape);
    assert_eq!(name(&h), before, "Escape keeps the old name");
    double(&mut h);
    h.event(Event::Key {
        key: Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    });
    h.event(Event::Text("Hero".into()));
    h.run_steps(1);
    key(&mut h, Key::Enter);
    assert_eq!(name(&h), "Hero");
}

#[test]
fn the_eye_and_lock_buttons_hide_and_lock_one_layer() {
    let mut h = harness();
    let (A, _, _) = layout(&mut h);
    let flag = |h: &Harness<'_, App>, f: &str| h.state().tabs[0].doc.node(A).unwrap().data.flag(f);
    // The buttons show while the row is hovered.
    let pos = at(&h, &row(A));
    h.event(Event::PointerMoved(pos));
    h.run_steps(2);
    click_on(&mut h, &format!("layers.{A}.visible"), Modifiers::NONE);
    assert_eq!(flag(&h, "visible"), Some(false));
    // Hidden, the eye stays without hovering.
    h.event(Event::PointerMoved(Pos2::new(5.0, 5.0)));
    h.run_steps(2);
    click_on(&mut h, &format!("layers.{A}.visible"), Modifiers::NONE);
    assert_eq!(flag(&h, "visible"), Some(true));
    h.event(Event::PointerMoved(pos));
    h.run_steps(2);
    click_on(&mut h, &format!("layers.{A}.lock"), Modifiers::NONE);
    assert_eq!(flag(&h, "locked"), Some(true));
}

#[test]
fn dragging_a_row_onto_a_frame_moves_the_layer_into_it_in_place() {
    let mut h = harness();
    let (A, _, F) = layout(&mut h);
    let before = figstudio_render::node_bounds(&h.state().tabs[0].doc, A).unwrap();
    let (from, to) = (at(&h, &row(A)), at(&h, &row(F)));
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(1);
    for i in 1..=6 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 6.0)));
        h.run_steps(1);
    }
    h.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(2);
    let tab = &h.state().tabs[0];
    assert_eq!(tab.doc.node(A).unwrap().parent, Some(F));
    assert_eq!(
        tab.doc.children(F).last(),
        Some(&A),
        "at the top of the frame"
    );
    assert_eq!(
        figstudio_render::node_bounds(&tab.doc, A).unwrap(),
        before,
        "in place"
    );
    assert_eq!(tab.selection, Some(A));
}

#[test]
fn the_tree_lists_the_frontmost_layer_first() {
    let mut h = harness();
    // Made in this order, so F is in front of B, which is in front of A.
    let (A, B, F) = layout(&mut h);
    let y = |h: &Harness<'_, App>, id: Guid| at(h, &row(id)).y;
    assert!(y(&h, F) < y(&h, B) && y(&h, B) < y(&h, A), "front first");
    // Dropping A just above F (in the list) brings it to the front.
    let page = h.state().tabs[0].page;
    let ops = crate::layers_edit::drop_ops(
        &h.state().tabs[0].doc,
        &[A],
        crate::layers_edit::Drop::Above(F),
        &|_| false,
    );
    crate::editor::apply_tab_ops(&mut h.state_mut().tabs[0], &ops, None, None).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().tabs[0].doc.children(page).last(), Some(&A));
    assert!(y(&h, A) < y(&h, F), "and to the top of the list");
}

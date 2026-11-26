//! Canvas gestures on the real app, frame by frame: moving and resizing the whole selection,
//! Alt-drag copies, dragging an unselected layer, the rotate corner, snapping and nudging.
#![expect(non_snake_case, reason = "A and B name the two rectangles")]
use egui::{Event, Key, Modifiers, PointerButton};

use super::multi_select::layout;
use super::*;

fn screen(h: &Harness<'_, App>, x: f64, y: f64) -> Pos2 {
    let tab = &h.state().tabs[0];
    tab.view.to_screen(
        tab.canvas_rect.expect("laid out").min,
        Pos2::new(x as f32, y as f32),
    )
}

/// Press at page point `from`, move to `to` over several frames, release, with `mods` held.
fn gesture(h: &mut Harness<'_, App>, from: (f64, f64), to: (f64, f64), mods: Modifiers) {
    let (from, to) = (screen(h, from.0, from.1), screen(h, to.0, to.1));
    let mut c = Vec::new();
    h.event(Event::ModifiersChanged(mods));
    h.event(Event::PointerMoved(from));
    step(h, &mut c);
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: mods,
    });
    step(h, &mut c);
    for i in 1..=6 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 6.0)));
        step(h, &mut c);
    }
    h.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: mods,
    });
    step(h, &mut c);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    step(h, &mut c);
}

fn xy(h: &Harness<'_, App>, id: Guid) -> (f64, f64) {
    let (x, y, _, _) = crate::editor::local_xywh(&h.state().tabs[0].doc.node(id).unwrap().data);
    (x, y)
}

fn wh(h: &Harness<'_, App>, id: Guid) -> (f64, f64) {
    let (_, _, w, hh) = crate::editor::local_xywh(&h.state().tabs[0].doc.node(id).unwrap().data);
    (w, hh)
}

fn undo(h: &mut Harness<'_, App>) {
    let tab = &mut h.state_mut().tabs[0];
    crate::editor::undo_tab(tab).unwrap();
    h.run_steps(2);
}

#[test]
fn dragging_moves_every_selected_layer_as_one_undo_step() {
    let mut h = harness();
    let (A, B, _) = layout(&mut h);
    h.state_mut().tabs[0].select_all_of(&[A, B]);
    h.run_steps(2);
    // Far from the others (below), so nothing snaps.
    gesture(&mut h, (30.0, 20.0), (30.0, 320.0), Modifiers::NONE);
    assert_eq!(xy(&h, A), (0.0, 300.0));
    assert_eq!(xy(&h, B), (100.0, 300.0));
    undo(&mut h);
    assert_eq!(xy(&h, A), (0.0, 0.0));
    assert_eq!(xy(&h, B), (100.0, 0.0));
}

#[test]
fn alt_drag_moves_a_copy_and_leaves_the_original() {
    let mut h = harness();
    let (A, _, _) = layout(&mut h);
    h.state_mut().tabs[0].select(Some(A));
    h.run_steps(2);
    let page = h.state().tabs[0].page;
    let before = h.state().tabs[0].doc.children(page).len();
    gesture(&mut h, (30.0, 20.0), (30.0, 320.0), Modifiers::ALT);
    let tab = &h.state().tabs[0];
    assert_eq!(tab.doc.children(page).len(), before + 1);
    assert_eq!(xy(&h, A), (0.0, 0.0), "the original stays");
    let copy = tab.selection.expect("the copy is selected");
    assert_ne!(copy, A);
    assert_eq!(xy(&h, copy), (0.0, 300.0));
    undo(&mut h);
    assert_eq!(
        h.state().tabs[0].doc.children(page).len(),
        before,
        "one undo removes the copy"
    );
}

#[test]
fn dragging_an_unselected_layer_selects_and_moves_it() {
    let mut h = harness();
    let (_, B, _) = layout(&mut h);
    gesture(&mut h, (130.0, 20.0), (130.0, 320.0), Modifiers::NONE);
    assert_eq!(h.state().tabs[0].selection, Some(B));
    assert_eq!(xy(&h, B), (100.0, 300.0));
}

#[test]
fn the_right_handle_scales_several_layers_inside_their_box() {
    let mut h = harness();
    let (A, B, _) = layout(&mut h);
    h.state_mut().tabs[0].select_all_of(&[A, B]);
    h.run_steps(2);
    // The box is x 0..160; its right handle at (160, 20) dragged to 320 doubles it.
    gesture(&mut h, (160.0, 20.0), (320.0, 20.0), Modifiers::NONE);
    assert_eq!((xy(&h, A), wh(&h, A)), ((0.0, 0.0), (120.0, 40.0)));
    assert_eq!((xy(&h, B), wh(&h, B)), ((200.0, 0.0), (120.0, 40.0)));
}

#[test]
fn dragging_just_outside_a_corner_rotates() {
    let mut h = harness();
    let (A, _, _) = layout(&mut h);
    h.state_mut().tabs[0].select(Some(A));
    h.run_steps(2);
    // From just outside the top-right corner (8 points out), a quarter turn clockwise about
    // the centre (30, 20).
    let off = 8.0 / f64::from(h.state().tabs[0].view.zoom);
    let (vx, vy) = (30.0 + off, -20.0 - off);
    gesture(
        &mut h,
        (60.0 + off, -off),
        (30.0 - vy, 20.0 + vx),
        Modifiers::NONE,
    );
    let rot = crate::editor::commands::current_rotation_of(&h.state().tabs[0].doc, A);
    assert!((rot - 90.0).abs() < 1.0, "{rot}");
    // It turned about its centre.
    let b = figstudio_render::node_bounds(&h.state().tabs[0].doc, A).unwrap();
    assert!(
        (b.center().x - 30.0).abs() < 0.6 && (b.center().y - 20.0).abs() < 0.6,
        "{b:?}"
    );
}

#[test]
fn a_move_snaps_to_a_neighbour_edge_unless_snapping_is_off() {
    let mut h = harness();
    let (A, _, _) = layout(&mut h);
    h.state_mut().tabs[0].select(Some(A));
    h.run_steps(2);
    // A's right edge (60) dragged to 99: one unit short of B's left edge (100).
    let from = (30.0, 20.0);
    gesture(&mut h, from, (69.0, 20.0), Modifiers::NONE);
    assert_eq!(xy(&h, A), (40.0, 0.0), "snapped to B");
    undo(&mut h);
    h.state_mut().prefs.snapping = false;
    gesture(&mut h, from, (69.0, 20.0), Modifiers::NONE);
    assert_eq!(xy(&h, A), (39.0, 0.0), "no snapping");
}

#[test]
fn arrow_keys_nudge_by_one_and_shift_by_ten() {
    let mut h = harness();
    let (A, B, _) = layout(&mut h);
    h.state_mut().tabs[0].select_all_of(&[A, B]);
    h.run_steps(2);
    let key = |h: &mut Harness<'_, App>, key: Key, modifiers: Modifiers| {
        h.event(Event::ModifiersChanged(modifiers));
        h.event(Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
        h.run_steps(2);
        h.event(Event::ModifiersChanged(Modifiers::NONE));
    };
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    key(&mut h, Key::ArrowDown, Modifiers::SHIFT);
    assert_eq!(xy(&h, A), (1.0, 10.0));
    assert_eq!(xy(&h, B), (101.0, 10.0));
}

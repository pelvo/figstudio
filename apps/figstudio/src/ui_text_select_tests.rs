use super::*;
use egui::{Key, Modifiers};
use figstudio_render::{absolute_transform, kurbo::Point};

const TEXT: Guid = Guid(1, 5);

pub(super) fn text_harness() -> Harness<'static, App> {
    let mut h = harness_fixture("noise_and_grain.fig", 1.0 / 60.0);
    h.state_mut().tabs[0].fit_node = Some(TEXT);
    h.run_steps(90);
    h
}

pub(super) fn glyph_at(h: &Harness<'_, App>, index: usize) -> Pos2 {
    let tab = &h.state().tabs[0];
    let data = &tab.doc.node(TEXT).expect("public text").data;
    let glyph = &data
        .path(&["derivedTextData", "glyphs"])
        .expect("glyphs")
        .items()[index];
    let p = glyph.get("position").expect("position");
    let local = Point::new(
        f64::from(p.num("x").expect("x")) + 0.1,
        f64::from(p.num("y").expect("y") - glyph.num("fontSize").expect("size") * 0.5),
    );
    let page = absolute_transform(&tab.doc, TEXT) * local;
    tab.view.to_screen(
        tab.canvas_rect.expect("canvas").min,
        Pos2::new(page.x as f32, page.y as f32),
    )
}

pub(super) fn double_click(h: &mut Harness<'_, App>, c: &mut Vec<String>, at: Pos2) {
    h.run_steps(60); // One second exceeds egui's double AND triple-click windows.
    h.event(Event::PointerMoved(at));
    step(h, c);
    for _ in 0..2 {
        press(h, at, PointerButton::Primary, true);
        step(h, c);
        press(h, at, PointerButton::Primary, false);
        step(h, c);
    }
}

pub(super) fn key(h: &mut Harness<'_, App>, c: &mut Vec<String>, key: Key, modifiers: Modifiers) {
    if key == Key::C && modifiers == Modifiers::COMMAND {
        h.event(Event::Copy);
        step(h, c);
        return;
    }
    h.event(Event::ModifiersChanged(modifiers));
    step(h, c);
    for pressed in [true, false] {
        h.event(Event::Key {
            key,
            pressed,
            modifiers,
            repeat: false,
            physical_key: None,
        });
        step(h, c);
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    step(h, c);
}

#[test]
fn double_click_enters_at_a_glyph_and_empty_range_copies_nothing() {
    let (mut h, mut c) = (text_harness(), Vec::new());
    let at = glyph_at(&h, 2);
    double_click(&mut h, &mut c, at);
    let selection = h.state().tabs[0].text_select.expect("text selection mode");
    assert_eq!(
        (selection.node, selection.anchor, selection.head),
        (TEXT, 2, 2)
    );
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::Text
    );
    key(&mut h, &mut c, Key::C, Modifiers::COMMAND);
    assert!(c.is_empty());
    h.run_steps(30);
    let at = glyph_at(&h, 3);
    h.event(Event::PointerMoved(at));
    step(&mut h, &mut c);
    press(&h, at, PointerButton::Primary, true);
    step(&mut h, &mut c);
    press(&h, at, PointerButton::Primary, false);
    step(&mut h, &mut c);
    let s = h.state().tabs[0]
        .text_select
        .expect("click inside stays in mode");
    assert_eq!((s.anchor, s.head), (3, 3));
}

#[test]
fn dragging_copies_a_substring_and_select_all_extends_it() {
    let (mut h, mut c) = (text_harness(), Vec::new());
    let from = glyph_at(&h, 0);
    let to = glyph_at(&h, 4);
    double_click(&mut h, &mut c, from);
    press(&h, from, PointerButton::Primary, true);
    step(&mut h, &mut c);
    h.event(Event::PointerMoved(to));
    step(&mut h, &mut c);
    press(&h, to, PointerButton::Primary, false);
    step(&mut h, &mut c);
    assert!(h.state().tabs[0].marquee.rect().is_none());
    key(&mut h, &mut c, Key::C, Modifiers::COMMAND);
    assert_eq!(c, ["Text"]);
    key(&mut h, &mut c, Key::A, Modifiers::COMMAND);
    key(&mut h, &mut c, Key::C, Modifiers::COMMAND);
    assert_eq!(c, ["Text", "Text with Noise"]);
}

#[test]
fn escape_exits_mode_before_clearing_the_layer() {
    let (mut h, mut c) = (text_harness(), Vec::new());
    let at = glyph_at(&h, 0);
    double_click(&mut h, &mut c, at);
    key(&mut h, &mut c, Key::Escape, Modifiers::NONE);
    assert!(h.state().tabs[0].text_select.is_none());
    assert_eq!(h.state().tabs[0].selection, Some(TEXT));
    key(&mut h, &mut c, Key::C, Modifiers::COMMAND);
    assert_eq!(c, ["Text with Noise"]);
    c.clear();
    key(&mut h, &mut c, Key::Escape, Modifiers::NONE);
    key(&mut h, &mut c, Key::C, Modifiers::COMMAND);
    assert!(c.is_empty());
}

#[test]
fn missing_glyphs_select_all_and_non_text_never_fits() {
    let (mut h, mut c) = (harness_fixture(FIXTURE, 1.0 / 60.0), Vec::new());
    let text = Guid(1, 42);
    let at = centre_of(&h, text);
    double_click(&mut h, &mut c, at);
    let selection = h.state().tabs[0].text_select.expect("whole-text fallback");
    assert_eq!((selection.anchor, selection.head), (0, 3));
    key(&mut h, &mut c, Key::C, Modifiers::COMMAND);
    assert_eq!(c, ["ABC"]);
    let before = h.state().tabs[0].view;
    let at = centre_of(&h, Guid(1, 34));
    double_click(&mut h, &mut c, at);
    h.run_steps(90);
    assert!(h.state().tabs[0].text_select.is_none());
    assert_eq!(h.state().tabs[0].view.zoom, before.zoom);
    assert_eq!(h.state().tabs[0].view.pan, before.pan);
}

#[test]
fn clicking_elsewhere_and_changing_selection_clear_text_mode() {
    let (mut h, mut c) = (text_harness(), Vec::new());
    let at = glyph_at(&h, 1);
    double_click(&mut h, &mut c, at);
    let elsewhere =
        h.state().tabs[0].canvas_rect.expect("canvas").left_top() + egui::vec2(8.0, 8.0);
    h.event(Event::PointerMoved(elsewhere));
    step(&mut h, &mut c);
    press(&h, elsewhere, PointerButton::Primary, true);
    step(&mut h, &mut c);
    press(&h, elsewhere, PointerButton::Primary, false);
    step(&mut h, &mut c);
    assert!(h.state().tabs[0].text_select.is_none());
    assert_ne!(h.state().tabs[0].selection, Some(TEXT));
    double_click(&mut h, &mut c, at);
    assert!(h.state().tabs[0].text_select.is_some());
    h.state_mut().tabs[0].select(Some(Guid(1, 8)));
    assert!(h.state().tabs[0].text_select.is_none());
    double_click(&mut h, &mut c, at);
    assert!(h.state().tabs[0].text_select.is_some());
    h.state_mut().tabs[0].set_page(Guid(0, 0));
    assert!(h.state().tabs[0].text_select.is_none());
}

#[test]
fn select_all_leaves_find_field_shortcuts_alone_with_or_without_canvas_mode() {
    for mode in [false, true] {
        let (mut h, mut c) = (text_harness(), Vec::new());
        if mode {
            let at = glyph_at(&h, 2);
            double_click(&mut h, &mut c, at);
        }
        h.state_mut().find.query = "unmatched query".into();
        h.state_mut().find.show();
        h.run_steps(30);
        // Move the field's caret to the end before asking Cmd+A to select it.
        key(&mut h, &mut c, Key::ArrowRight, Modifiers::NONE);
        key(&mut h, &mut c, Key::A, Modifiers::COMMAND);
        h.event(Event::Text("replacement".into()));
        step(&mut h, &mut c);
        assert_eq!(h.state().find.query, "replacement");
        if mode {
            let s = h.state().tabs[0]
                .text_select
                .expect("canvas range unchanged");
            assert_eq!((s.anchor, s.head), (2, 2));
        }
    }
}

#[test]
fn reverse_drag_and_remapped_select_all_copy_the_expected_text() {
    let (mut h, mut c) = (text_harness(), Vec::new());
    let from = glyph_at(&h, 4);
    let to = glyph_at(&h, 0);
    double_click(&mut h, &mut c, from);
    press(&h, from, PointerButton::Primary, true);
    step(&mut h, &mut c);
    h.event(Event::PointerMoved(to));
    step(&mut h, &mut c);
    press(&h, to, PointerButton::Primary, false);
    step(&mut h, &mut c);
    key(&mut h, &mut c, Key::C, Modifiers::COMMAND);
    assert_eq!(c, ["Text"]);
    h.state_mut().prefs.keymap.set(
        crate::keymap::Binding::SelectAll,
        Some(egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::D)),
    );
    key(&mut h, &mut c, Key::D, Modifiers::COMMAND);
    key(&mut h, &mut c, Key::C, Modifiers::COMMAND);
    assert_eq!(c, ["Text", "Text with Noise"]);
}

#[test]
fn the_selection_range_paints_an_overlay_that_tracks_it() {
    let (mut h, mut c) = (text_harness(), Vec::new());
    let from = glyph_at(&h, 0);
    double_click(&mut h, &mut c, from);
    let quads = |h: &Harness<'_, App>| -> usize {
        h.output()
            .shapes
            .iter()
            .filter_map(|cs| {
                let s = &cs.shape;
                matches!(s, egui::Shape::Path(p) if p.closed
                    && p.fill == crate::theme::ACCENT.gamma_multiply(0.25))
                .then_some(())
            })
            .count()
    };
    assert_eq!(quads(&h), 0, "an empty range paints nothing");
    let to = glyph_at(&h, 4);
    press(&h, from, PointerButton::Primary, true);
    step(&mut h, &mut c);
    h.event(Event::PointerMoved(to));
    step(&mut h, &mut c);
    press(&h, to, PointerButton::Primary, false);
    step(&mut h, &mut c);
    assert_eq!(quads(&h), 1, "one line selected paints one quad");
    assert!(h.state().tabs[0].marquee.rect().is_none());
}

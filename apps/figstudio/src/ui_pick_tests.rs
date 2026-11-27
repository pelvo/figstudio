use super::*;
use egui::{Key, Modifiers};
use egui_kittest::kittest::NodeT;
use figstudio_fig::{Document, Node, Obj, Val};
use std::sync::Arc;

pub const FRAME: Guid = Guid(99, 2);
pub const GROUP: Guid = Guid(99, 3);
pub const NEST: Guid = Guid(99, 4);
pub const TEXT: Guid = Guid(99, 5);
pub const SIBLING: Guid = Guid(99, 6);

pub(super) fn object(fields: &[(&str, Val)]) -> Val {
    Val::Obj(fields.iter().fold(Obj::new(), |mut o, (key, value)| {
        o.insert(key, value.clone());
        o
    }))
}

pub fn nested() -> Document {
    let mut doc = Document::open(fixture(FIXTURE)).unwrap();
    let nodes = [
        (1, 0, "CANVAS"),
        (2, 1, "FRAME"),
        (3, 2, "GROUP"),
        (4, 3, "GROUP"),
        (5, 4, "TEXT"),
        (6, 4, "RECTANGLE"),
        (7, 1, "GROUP"),
        (8, 7, "RECTANGLE"),
    ]
    .into_iter()
    .map(|(id, parent, kind)| {
        let (w, h) = if id == 2 {
            (300.0, 200.0)
        } else {
            (80.0, 80.0)
        };
        Node::from_data(object(&[
            ("guid", Guid(99, id).to_val()),
            ("type", Val::Str(kind.into())),
            (
                "parentIndex",
                object(&[
                    ("guid", Guid(99, parent).to_val()),
                    ("position", Val::Str(format!("{id:03}"))),
                ]),
            ),
            (
                "size",
                object(&[("x", Val::Float(w)), ("y", Val::Float(h))]),
            ),
            (
                "transform",
                object(&[
                    ("m00", Val::Float(1.0)),
                    ("m11", Val::Float(1.0)),
                    (
                        "m02",
                        Val::Float(if id == 6 {
                            100.0
                        } else if id == 7 {
                            400.0
                        } else {
                            0.0
                        }),
                    ),
                ]),
            ),
            (
                "textData",
                object(&[("characters", Val::Str("sample".into()))]),
            ),
            ("frameMaskDisabled", Val::Bool(true)),
        ]))
        .unwrap()
    })
    .collect();
    doc.set_nodes(nodes).unwrap();
    doc
}

pub fn nested_harness() -> Harness<'static, App> {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    h.state_mut().reduce_motion = true;
    let tab = &mut h.state_mut().tabs[0];
    tab.doc = Arc::new(nested());
    tab.set_page(Guid(99, 1));
    tab.fit_pending = true;
    h.run_steps(4);
    h
}

pub fn click(h: &mut Harness<'_, App>, at: Pos2, modifiers: Modifiers) {
    h.run_steps(60);
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        });
        h.step();
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.step();
}

#[test]
fn parent_first_click_and_top_frame_exception() {
    let mut h = nested_harness();
    let at = centre_of(&h, TEXT);
    click(&mut h, at, Modifiers::NONE);
    assert_eq!(h.state().tabs[0].selection, Some(GROUP));
    let at = centre_of(&h, Guid(99, 8));
    click(&mut h, at, Modifiers::NONE);
    assert_eq!(h.state().tabs[0].selection, Some(Guid(99, 7)));
}

#[test]
fn double_click_drills_one_level_repeatedly_then_enters_text() {
    let (mut h, mut c) = (nested_harness(), Vec::new());
    let at = centre_of(&h, TEXT);
    click(&mut h, at, Modifiers::NONE);
    text_select::double_click(&mut h, &mut c, at);
    assert_eq!(h.state().tabs[0].selection, Some(NEST));
    assert!(h.state().tabs[0].text_select.is_none());
    text_select::double_click(&mut h, &mut c, at);
    assert_eq!(h.state().tabs[0].selection, Some(TEXT));
    assert!(h.state().tabs[0].text_select.is_some());
}

#[test]
fn sibling_click_keeps_drilled_depth() {
    let mut h = nested_harness();
    h.state_mut().tabs[0].select(Some(TEXT));
    let at = centre_of(&h, SIBLING);
    click(&mut h, at, Modifiers::NONE);
    assert_eq!(h.state().tabs[0].selection, Some(SIBLING));
    assert!(h.state().tabs[0].text_select.is_none());
}

#[test]
fn command_click_deep_selects() {
    let mut h = nested_harness();
    let at = centre_of(&h, TEXT);
    click(&mut h, at, Modifiers::COMMAND);
    assert_eq!(h.state().tabs[0].selection, Some(TEXT));
    assert!(h.state().tabs[0].text_select.is_none());
}

#[test]
fn enter_and_shift_enter_navigate_without_leaving_page() {
    let (mut h, mut c) = (nested_harness(), Vec::new());
    h.state_mut().tabs[0].select(Some(GROUP));
    text_select::key(&mut h, &mut c, Key::Enter, Modifiers::NONE);
    assert_eq!(h.state().tabs[0].selection, Some(NEST));
    text_select::key(&mut h, &mut c, Key::Enter, Modifiers::SHIFT);
    assert_eq!(h.state().tabs[0].selection, Some(GROUP));
    text_select::key(&mut h, &mut c, Key::Enter, Modifiers::SHIFT);
    text_select::key(&mut h, &mut c, Key::Enter, Modifiers::SHIFT);
    assert_eq!(h.state().tabs[0].selection, Some(FRAME));
}

#[test]
fn navigation_is_rebindable_and_waits_for_find_or_command_fields() {
    let (mut h, mut c) = (nested_harness(), Vec::new());
    h.state_mut().tabs[0].select(Some(GROUP));
    h.state_mut().prefs.keymap.set(
        crate::keymap::Binding::SelectChild,
        Some(egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::D)),
    );
    text_select::key(&mut h, &mut c, Key::Enter, Modifiers::NONE);
    assert_eq!(h.state().tabs[0].selection, Some(GROUP));
    h.state_mut().find.show();
    h.run_steps(30);
    text_select::key(&mut h, &mut c, Key::D, Modifiers::COMMAND);
    assert_eq!(h.state().tabs[0].selection, Some(GROUP));
    h.state_mut().find.hide();
    h.run_steps(30);
    text_select::key(&mut h, &mut c, Key::D, Modifiers::COMMAND);
    assert_eq!(h.state().tabs[0].selection, Some(NEST));
    let ctx = h.ctx.clone();
    h.state_mut().command.toggle(&ctx);
    h.run_steps(30);
    text_select::key(&mut h, &mut c, Key::D, Modifiers::COMMAND);
    assert_eq!(h.state().tabs[0].selection, Some(NEST));
}

#[test]
fn non_text_double_click_never_fits() {
    let (mut h, mut c) = (nested_harness(), Vec::new());
    let at = centre_of(&h, TEXT);
    click(&mut h, at, Modifiers::NONE);
    let before = h.state().tabs[0].view;
    text_select::double_click(&mut h, &mut c, at);
    h.run_steps(90);
    let tab = &h.state().tabs[0];
    assert_eq!(tab.view.zoom, before.zoom);
    assert_eq!(tab.view.pan, before.pan);
    assert!(tab.fit_node.is_none());
}

#[test]
fn hover_matches_click_with_and_without_command() {
    let mut h = nested_harness();
    let at = centre_of(&h, TEXT);
    h.event(Event::PointerMoved(at));
    h.step();
    assert_eq!(h.state().tabs[0].hover, Some(GROUP));
    h.event(Event::ModifiersChanged(Modifiers::COMMAND));
    h.step();
    assert_eq!(h.state().tabs[0].hover, Some(TEXT));
}

#[test]
fn context_click_uses_the_same_parent_first_rule() {
    let (mut h, mut c) = (nested_harness(), Vec::new());
    let at = centre_of(&h, TEXT);
    right_click(&mut h, &mut c, at);
    assert_eq!(h.state().tabs[0].selection, Some(GROUP));
}

#[test]
fn pure_picker_keeps_context_and_skips_only_the_top_frame() {
    let doc = nested();
    let chain = [FRAME, GROUP, NEST, TEXT];
    assert_eq!(crate::pick::click(&doc, &chain, None, false), Some(GROUP));
    assert_eq!(
        crate::pick::click(&doc, &chain, Some(NEST), false),
        Some(NEST)
    );
    assert_eq!(
        crate::pick::click(&doc, &chain, Some(SIBLING), false),
        Some(TEXT)
    );
    assert_eq!(crate::pick::click(&doc, &chain, None, true), Some(TEXT));
    assert_eq!(crate::pick::click(&doc, &[], None, false), None);
}

fn assert_jitter_double_click_drills_cleanly(
    h: &mut Harness<'_, App>,
    at: Pos2,
    outer: Guid,
    inner: Guid,
    jitter: f32,
) {
    primary_clicks(h, at, egui::Vec2::ZERO, 1);
    assert_eq!(h.state().tabs[0].selection, Some(outer));
    let before = figstudio_render::absolute_transform(&h.state().tabs[0].doc, outer)
        * figstudio_render::kurbo::Point::ZERO;

    h.run_steps(60);
    primary_clicks(h, at, egui::vec2(jitter, 0.0), 2);

    assert_eq!(
        h.state().tabs[0].selection,
        Some(inner),
        "double-click with {jitter} pt jitter must drill from {outer} to {inner}"
    );
    let after = figstudio_render::absolute_transform(&h.state().tabs[0].doc, outer)
        * figstudio_render::kurbo::Point::ZERO;
    assert!(
        (after - before).hypot() < 1e-3,
        "{jitter} pt double-click jitter must not move {outer}: {before:?} -> {after:?}"
    );
    assert!(
        !h.state().tabs[0].dirty_unsaved,
        "{jitter} pt double-click jitter must not dirty the document"
    );
}

#[test]
fn single_click_then_repeated_double_click_drills_within_triple_click_window() {
    let mut h = nested_harness();
    let at = centre_of(&h, TEXT);

    // Single click selects outer group.
    primary_clicks(&mut h, at, egui::Vec2::ZERO, 1);
    assert_eq!(h.state().tabs[0].selection, Some(GROUP));

    // Pause 400 ms (> 300 ms double-click delay, < 600 ms egui triple-click window), then double-click.
    h.run_steps(24);
    primary_clicks(&mut h, at, egui::Vec2::ZERO, 2);
    assert_eq!(
        h.state().tabs[0].selection,
        Some(NEST),
        "double-click 400 ms after single-click must drill one level deeper"
    );
    assert!(h.state().tabs[0].text_select.is_none());

    // A plain single-click 400 ms after a double-click must NOT accidentally drill further.
    h.run_steps(24);
    primary_clicks(&mut h, at, egui::Vec2::ZERO, 1);
    assert_eq!(
        h.state().tabs[0].selection,
        Some(NEST),
        "single-click 400 ms after double-click must stay on current level"
    );
    assert!(h.state().tabs[0].text_select.is_none());

    // Another double-click 400 ms later must drill a second level deeper into TEXT and enter text mode.
    h.run_steps(24);
    primary_clicks(&mut h, at, egui::Vec2::ZERO, 2);
    assert_eq!(
        h.state().tabs[0].selection,
        Some(TEXT),
        "second double-click within 600 ms window must drill to TEXT"
    );
    assert!(
        h.state().tabs[0].text_select.is_some(),
        "drilling onto TEXT via double-click must enter text selection"
    );
}

#[test]
fn double_click_drills_on_zoomed_out_layer_covered_by_resize_handles() {
    let mut h = nested_harness();
    // Zoom out so GROUP (80x80 page units) is 10x10 screen pt and its 10x10 resize handles cover its center.
    h.state_mut().tabs[0].zoom_centre(0.125);
    h.run_steps(4);

    let at = centre_of(&h, TEXT);
    primary_clicks(&mut h, at, egui::Vec2::ZERO, 1);
    assert_eq!(h.state().tabs[0].selection, Some(GROUP));

    // Wait > 1 s (cleanly outside triple-click window) and double-click with 0 px movement.
    for (expected, expect_text) in [(NEST, false), (TEXT, true)] {
        h.run_steps(60);
        primary_clicks(&mut h, at, egui::Vec2::ZERO, 2);
        assert_eq!(h.state().tabs[0].selection, Some(expected));
        assert_eq!(h.state().tabs[0].text_select.is_some(), expect_text);
        assert!(!h.state().tabs[0].dirty_unsaved);
    }
}

#[test]
fn double_click_with_sub_click_jitter_drills_without_moving_or_dirtying() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    h.state_mut().reduce_motion = true;
    h.run_steps(4);
    let at = centre_of(&h, Guid(1, 50));
    assert_jitter_double_click_drills_cleanly(&mut h, at, Guid(1, 53), Guid(1, 52), 4.0);
}

#[test]
fn double_click_at_exact_max_click_dist_drills_without_moving() {
    // Test both the layer body (Move threshold) and the inner half of the NW resize handle
    // (Resize threshold when inside_node) at exact `max_click_dist` (egui rejects `>`, not `>=`).
    for offset in [egui::vec2(40.0, 40.0), egui::vec2(1.0, 1.0)] {
        let mut h = nested_harness();
        let max_dist = h.ctx.options(|o| o.input_options.max_click_dist);
        h.state_mut().tabs[0].zoom_centre(1.0);
        h.run_steps(4);
        let tab = &h.state().tabs[0];
        let at = tab
            .view
            .to_screen(tab.canvas_rect.unwrap().min, Pos2::new(offset.x, offset.y));
        assert_jitter_double_click_drills_cleanly(&mut h, at, GROUP, NEST, max_dist);
    }
}

#[test]
fn synthetic_clicks_and_non_canvas_clicks_do_not_form_double_click_pairs() {
    let mut h = nested_harness();
    let at = centre_of(&h, TEXT);
    let (canvas_node, canvas_tree) = h
        .get_by_label_contains("Canvas, page")
        .accesskit_node()
        .locate();

    // Pointer click followed within max_double_click_delay by a synthetic AccessKit click on the
    // canvas (while the pointer still hovers `at`) must not pair, nor let a later pointer click pair.
    primary_clicks(&mut h, at, egui::Vec2::ZERO, 1);
    assert_eq!(h.state().tabs[0].selection, Some(GROUP));
    h.run_steps(4);
    h.event(Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            target_node: canvas_node,
            target_tree: canvas_tree,
            action: egui::accesskit::Action::Click,
            data: None,
        },
    ));
    h.step();
    assert_eq!(
        h.state().tabs[0].selection,
        Some(GROUP),
        "synthetic AccessKit click must not pair with prior pointer click"
    );
    h.run_steps(4);
    primary_clicks(&mut h, at, egui::Vec2::ZERO, 1);
    assert_eq!(
        h.state().tabs[0].selection,
        Some(GROUP),
        "pointer click must not pair across an intervening synthetic click"
    );

    // An intervening click outside the canvas (e.g., on the Properties panel) within the double-click
    // delay must break the pair.
    h.run_steps(60);
    primary_clicks(&mut h, at, egui::Vec2::ZERO, 1);
    assert_eq!(h.state().tabs[0].selection, Some(GROUP));
    let outside = h.state().tabs[0].canvas_rect.unwrap().right_bottom() + egui::vec2(20.0, -20.0);
    h.run_steps(2);
    primary_clicks(&mut h, outside, egui::Vec2::ZERO, 1);
    h.run_steps(2);
    primary_clicks(&mut h, at, egui::Vec2::ZERO, 1);
    assert_eq!(
        h.state().tabs[0].selection,
        Some(GROUP),
        "canvas clicks must not pair across a non-canvas pointer click"
    );
}

use super::*;
use egui::{Key, Modifiers};
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

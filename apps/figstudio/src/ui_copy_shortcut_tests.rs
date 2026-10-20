//! Clipboard events and focused fields through the real app.
use super::*;
use crate::{keymap::Binding, menu::Action};
use egui::{Key, Modifiers};

const TEXT: Guid = Guid(1, 42);

fn selected() -> Harness<'static, App> {
    let mut h = harness();
    h.state_mut().tabs[0].select(Some(TEXT));
    h
}

fn focus_find(h: &mut Harness<'_, App>, c: &mut Vec<String>) {
    h.state_mut().find.query = "field text".into();
    h.state_mut().find.show();
    h.run_steps(3);
    text_select::key(h, c, Key::ArrowRight, Modifiers::NONE);
    text_select::key(h, c, Key::A, Modifiers::COMMAND);
    assert!(h.ctx.egui_wants_keyboard_input());
}

#[test]
fn copy_event_copies_selected_layer_but_not_over_a_focused_find_field() {
    let (mut h, mut c) = (selected(), Vec::new());
    h.event(Event::Copy);
    step(&mut h, &mut c);
    assert_eq!(c, ["ABC"]);
    c.clear();
    focus_find(&mut h, &mut c);
    h.event(Event::Copy);
    step(&mut h, &mut c);
    assert_eq!(c, ["field text"], "the field owns clipboard copy");
    assert!(!h.state().toasts.iter().any(|t| t.text == "Copied text"));
}

#[test]
fn command_c_key_is_not_consumed_while_find_has_focus() {
    let (mut h, mut c) = (selected(), Vec::new());
    focus_find(&mut h, &mut c);
    h.event(Event::Key {
        key: Key::C,
        pressed: true,
        modifiers: Modifiers::COMMAND,
        repeat: false,
        physical_key: None,
    });
    step(&mut h, &mut c);
    assert!(c.is_empty(), "canvas copy must not run: {c:?}");
    assert!(
        h.ctx.input(|i| i.events.iter().any(|e| matches!(
            e,
            Event::Key {
                key: Key::C,
                pressed: true,
                ..
            }
        ))),
        "the field keeps the chord"
    );
    assert!(!h.state().toasts.iter().any(|t| t.text == "Copied text"));
}

#[test]
fn native_copy_and_select_all_leave_the_focused_field_alone() {
    let (mut h, mut c) = (selected(), Vec::new());
    h.state_mut().tabs[0].text_select = Some(crate::text_select::Selection {
        node: TEXT,
        anchor: 1,
        head: 2,
    });
    focus_find(&mut h, &mut c);
    let ctx = h.ctx.clone();
    h.state_mut()
        .apply_native(vec![Action::CopyText, Action::SelectAllText], &ctx);
    let tab = &h.state().tabs[0];
    assert!(tab.pending_copy.is_none());
    let s = tab.text_select.unwrap();
    assert_eq!((s.anchor, s.head), (1, 2));
}

#[test]
fn rebound_copy_uses_its_key_not_the_platform_copy_event() {
    let (mut h, mut c) = (selected(), Vec::new());
    h.state_mut().prefs.keymap.set(
        Binding::Copy,
        Some(egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::D)),
    );
    h.event(Event::Copy);
    step(&mut h, &mut c);
    assert!(c.is_empty());
    text_select::key(&mut h, &mut c, Key::D, Modifiers::COMMAND);
    assert_eq!(c, ["ABC"]);
}

#[test]
fn edit_copy_requires_nonempty_text_and_context_copy_matches_the_active_range() {
    fn enabled(items: &[crate::menu::Item]) -> bool {
        items
            .iter()
            .any(|i| (i.action == Some(Action::CopyText) && i.enabled) || enabled(&i.children))
    }
    let mut h = selected();
    assert!(enabled(&crate::menu::tree(h.state())));
    h.state_mut().tabs[0].select(Some(Guid(1, 34)));
    assert!(!enabled(&crate::menu::tree(h.state())));
    let tab = &mut h.state_mut().tabs[0];
    tab.select(Some(TEXT));
    tab.text_select = Some(crate::text_select::Selection {
        node: TEXT,
        anchor: 1,
        head: 1,
    });
    assert!(
        !enabled(&crate::menu::tree(h.state())),
        "an empty caret cannot copy"
    );
    let tab = &mut h.state_mut().tabs[0];
    tab.text_select.as_mut().unwrap().head = 3;
    assert!(enabled(&crate::menu::tree(h.state())));
}

#[test]
fn context_copy_uses_the_canvas_range_and_resolved_inner_text() {
    let mut h = selected();
    let tab = &mut h.state_mut().tabs[0];
    tab.text_select = Some(crate::text_select::Selection {
        node: TEXT,
        anchor: 1,
        head: 3,
    });
    let copied = |tab: &crate::tab::Tab, id| {
        crate::copy::items(tab, id)
            .into_iter()
            .find(|i| i.label == "Copy text")
            .map(|i| i.text)
    };
    assert_eq!(copied(tab, TEXT), Some("BC".into()));
    tab.select(Some(Guid(1, 38)));
    tab.set_inner(Some(
        figstudio_fig::resolve::InstancePath::new(vec![Guid(1, 38), TEXT]).unwrap(),
    ));
    assert_eq!(copied(tab, Guid(1, 38)), Some("XYZ".into()));
    tab.text_select = Some(crate::text_select::Selection {
        node: TEXT,
        anchor: 1,
        head: 3,
    });
    assert_eq!(copied(tab, Guid(1, 38)), tab.selected_text());
    assert_eq!(copied(tab, Guid(1, 38)), Some("YZ".into()));
}

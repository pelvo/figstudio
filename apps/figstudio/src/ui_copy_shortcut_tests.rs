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
    for attached in [false, true] {
        for action in [Action::CopyText, Action::SelectAllText] {
            assert!(!crate::menu_dispatch::native_owned(true, attached, action));
            #[cfg(any(target_os = "macos", windows))]
            assert!(
                crate::menu_native::accelerator(action, Some(&crate::keymap::Keymap::default()),)
                    .is_none()
            );
        }
    }
    let (mut h, mut c) = (selected(), Vec::new());
    h.state_mut().tabs[0].text_select = Some(crate::text_select::Selection {
        node: TEXT,
        anchor: 1,
        head: 2,
    });
    focus_find(&mut h, &mut c);
    h.event(Event::Copy);
    text_select::key(&mut h, &mut c, Key::A, Modifiers::COMMAND);
    assert!(c.is_empty());
    let s = h.state().tabs[0].text_select.unwrap();
    assert_eq!((s.anchor, s.head), (1, 2));
    h.event(Event::Copy);
    step(&mut h, &mut c);
    assert_eq!(c, ["field text"]);
    c.clear();
    h.ctx
        .memory_mut(|m| m.surrender_focus(m.focused().unwrap()));
    text_select::key(&mut h, &mut c, Key::A, Modifiers::COMMAND);
    h.event(Event::Copy);
    step(&mut h, &mut c);
    assert_eq!(c, ["ABC"]);
}

#[test]
fn native_menu_clicks_explicitly_copy_the_canvas_even_with_field_focus() {
    let (mut h, mut c) = (selected(), Vec::new());
    focus_find(&mut h, &mut c);
    let ctx = h.ctx.clone();
    h.state_mut().apply_native(vec![Action::CopyText], &ctx);
    assert_eq!(h.state().tabs[0].pending_copy.as_ref().unwrap().text, "ABC");
}

#[test]
fn properties_copy_and_preview_copy_use_full_text_but_canvas_copy_uses_range() {
    let (mut h, mut c) = (selected(), Vec::new());
    h.state_mut().tabs[0].text_select = Some(crate::text_select::Selection {
        node: TEXT,
        anchor: 1,
        head: 3,
    });
    step(&mut h, &mut c);
    click_label(&mut h, &mut c, "Copy");
    assert_eq!(c, ["ABC"]);
    let at = h
        .get_all_by_label("ABC")
        .map(|node| node.rect().center())
        .max_by(|a, b| a.x.total_cmp(&b.x))
        .unwrap();
    right_click(&mut h, &mut c, at);
    step(&mut h, &mut c);
    click_label(&mut h, &mut c, "Copy text");
    assert_eq!(c, ["ABC", "ABC"]);
    let at = centre_of(&h, TEXT);
    right_click(&mut h, &mut c, at);
    step(&mut h, &mut c);
    click_label(&mut h, &mut c, "Copy text");
    assert_eq!(c, ["ABC", "ABC", "BC"]);
}

#[test]
fn resolved_preview_copies_full_characters_even_with_an_empty_canvas_caret() {
    for head in [1, 3] {
        let mut app = selected();
        let mut tab = app.state_mut().tabs.remove(0);
        tab.select(Some(Guid(1, 38)));
        tab.set_inner(Some(
            figstudio_fig::resolve::InstancePath::new(vec![Guid(1, 38), TEXT]).unwrap(),
        ));
        tab.text_select = Some(crate::text_select::Selection {
            node: TEXT,
            anchor: 1,
            head,
        });
        let (id, _, data) = tab.text_layout().unwrap();
        let mut node = tab.doc.node(id).unwrap().clone();
        node.data = data;
        let mut h = Harness::builder().build_ui_state(
            move |ui, picked| crate::copy_menu::text_block(ui, &tab, &node, picked),
            None,
        );
        h.get_by_label("Copy").click();
        h.run();
        assert_eq!(h.state().as_ref().unwrap().text, "XYZ");
        *h.state_mut() = None;
        h.get_by_label("XYZ").click_secondary();
        h.run();
        h.get_by_label("Copy text").click();
        h.run();
        assert_eq!(h.state().as_ref().unwrap().text, "XYZ");
    }
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

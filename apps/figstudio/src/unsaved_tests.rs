//! The unsaved-edits question is modal: nothing else acts while it is up, and an update
//! restart (which quits) asks it too.

use egui_kittest::kittest::Queryable;
use figstudio_fig::Guid;

use crate::menu::Action;
use crate::unsaved::Pending;

/// The fixture with one edit, so its tab has unsaved edits.
fn edited() -> egui_kittest::Harness<'static, crate::App> {
    let mut h = crate::ui_tests::harness();
    let ctx = egui::Context::default();
    h.state_mut().tabs[0].select(Some(Guid(1, 25)));
    h.state_mut().apply(Action::DuplicateSelection, &ctx);
    assert!(h.state().tabs[0].dirty_unsaved);
    h
}

#[test]
fn shortcuts_and_menu_commands_wait_while_the_question_is_up() {
    let mut h = edited();
    let ctx = egui::Context::default();
    h.state_mut().apply(Action::Quit, &ctx);
    h.run_steps(2);
    let nodes = h.state().tabs[0].doc.nodes.len();
    let revision = h.state().tabs[0].doc.revision;

    // ⌘W would swap the quit question for a close question.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::W);
    h.run_steps(2);
    assert_eq!(h.state().unsaved, Some(Pending::Quit));

    // The native macOS menu reaches `apply` while the modal is up.
    h.state_mut().apply(Action::DeleteSelection, &ctx);
    h.state_mut().apply(Action::CloseTab, &ctx);
    h.run_steps(2);
    assert_eq!(h.state().unsaved, Some(Pending::Quit));
    assert_eq!(h.state().tabs.len(), 1);
    assert_eq!(h.state().tabs[0].doc.nodes.len(), nodes);
    assert_eq!(
        h.state().tabs[0].doc.revision,
        revision,
        "no edit behind the question"
    );
}

#[test]
fn escape_cancels_the_question_and_goes_no_further() {
    let mut h = edited();
    let selection = h.state().tabs[0].selection;
    assert!(selection.is_some());
    let ctx = egui::Context::default();
    h.state_mut().apply(Action::CloseTab, &ctx);
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().unsaved, None);
    assert_eq!(h.state().tabs.len(), 1);
    assert_eq!(
        h.state().tabs[0].selection,
        selection,
        "the Escape that cancels must not also clear the selection"
    );
}

#[test]
fn an_update_restart_asks_about_unsaved_edits_before_scheduling_the_restart() {
    let mut h = edited();
    let ctx = egui::Context::default();
    h.state_mut().update_ask(
        crate::update_view::Ask::Restart("/nonexistent/figstudio.app".into()),
        &ctx,
    );
    assert_eq!(h.state().unsaved, Some(Pending::Quit));
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert_eq!(h.state().unsaved, None);
    assert!(!h.state().quit_confirmed);
}

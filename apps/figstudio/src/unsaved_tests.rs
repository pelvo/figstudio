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

fn find_action(items: &[crate::menu::Item], action: Action) -> Option<&crate::menu::Item> {
    items.iter().find_map(|item| {
        if item.action == Some(action) {
            Some(item)
        } else {
            find_action(&item.children, action)
        }
    })
}

#[test]
fn browser_disables_quit_and_tracks_unsaved_edits() {
    let mut h = edited();
    let ctx = egui::Context::default();
    assert!(
        !find_action(&crate::menu::tree_for(h.state(), true), Action::Quit)
            .unwrap()
            .enabled
    );
    assert!(
        find_action(&crate::menu::tree_for(h.state(), false), Action::Quit)
            .unwrap()
            .enabled
    );
    h.state_mut().quit_for(&ctx, true);
    assert_eq!(h.state().unsaved, None);
    assert!(!h.state().quit_confirmed);

    assert!(h.state().any_unsaved());
    h.state_mut().apply(Action::SaveCopy, &ctx);
    assert!(h.state().any_unsaved());
    h.state_mut().apply(Action::Undo, &ctx);
    assert!(!h.state().any_unsaved());
    h.state_mut().apply(Action::Redo, &ctx);
    assert!(h.state().any_unsaved());
    h.state_mut().apply(Action::CloseTab, &ctx);
    h.run_steps(2);
    h.get_by_label("Discard edits").click();
    h.run_steps(2);
    assert!(!h.state().any_unsaved());
}

#[test]
fn canvas_gizmos_do_not_act_behind_the_unsaved_question() {
    let mut h = edited();
    let ctx = h.ctx.clone();
    h.state_mut()
        .apply(Action::SelectTool(crate::canvas::Tool::Rectangle), &ctx);
    h.state_mut().apply(Action::CloseTab, &ctx);
    h.run_steps(2);
    let nodes_before = h.state().tabs[0].doc.nodes.len();
    let cancel = h.get_by_label("Cancel").rect().center();

    h.event(egui::Event::PointerMoved(cancel));
    crate::ui_tests::press(&h, cancel, egui::PointerButton::Primary, true);
    h.step();
    assert!(
        h.state().tabs[0].edit_drag.is_none(),
        "creation drag must not start behind the unsaved-edits modal"
    );
    h.event(egui::Event::PointerMoved(cancel + egui::vec2(60.0, 40.0)));
    h.step();
    crate::ui_tests::press(
        &h,
        cancel + egui::vec2(60.0, 40.0),
        egui::PointerButton::Primary,
        false,
    );
    h.run_steps(2);
    assert_eq!(h.state().tabs[0].doc.nodes.len(), nodes_before);

    // Same-frame click on Cancel dismisses the modal without triggering the canvas underneath.
    h.event(egui::Event::PointerMoved(cancel));
    crate::ui_tests::press(&h, cancel, egui::PointerButton::Primary, true);
    h.step();
    crate::ui_tests::press(&h, cancel, egui::PointerButton::Primary, false);
    h.run_steps(2);
    assert_eq!(h.state().unsaved, None);
    assert_eq!(h.state().tabs[0].doc.nodes.len(), nodes_before);
}

#[test]
fn restart_from_update_notes_closes_notes_so_cancel_is_clickable() {
    let mut h = edited();
    let rel = crate::update::Release {
        version: semver::Version::new(9, 9, 9),
        page: "https://example.invalid/release".into(),
        app: None,
        notes: vec![],
    };
    h.state_mut().updater.state =
        crate::update::State::Installed(rel, "/nonexistent/figstudio.app".into());
    h.state_mut().updater.notes_open = true;
    h.run_steps(3);
    h.get_by_label("Restart now").click();
    h.run_steps(3);
    assert_eq!(h.state().unsaved, Some(Pending::Quit));

    let cancel = h.get_by_label("Cancel").rect().center();
    h.event(egui::Event::PointerMoved(cancel));
    crate::ui_tests::press(&h, cancel, egui::PointerButton::Primary, true);
    crate::ui_tests::press(&h, cancel, egui::PointerButton::Primary, false);
    h.run_steps(2);
    assert_eq!(
        h.state().unsaved,
        None,
        "pointer click on Cancel must dismiss the unsaved-edits modal"
    );
    assert!(!h.state().updater.notes_open);
    assert!(!h.state().quit_confirmed);
}

#[test]
fn unsaved_modal_stays_above_command_bar_and_quit_button_is_clickable() {
    let mut h = edited();
    let ctx = h.ctx.clone();
    // Show and cancel the unsaved modal once first (so its layer already exists in egui's
    // order list, as after cancelling a prior close), then open Command Bar and Quit.
    h.state_mut().apply(Action::CloseTab, &ctx);
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert_eq!(h.state().unsaved, None);

    h.state_mut().apply(Action::CommandBar, &ctx);
    h.run_steps(3);
    assert!(h.state().command.is_open());

    h.state_mut().apply(Action::Quit, &ctx);
    h.run_steps(3);
    assert_eq!(h.state().unsaved, Some(Pending::Quit));

    let quit_btn = h.get_by_label("Quit without saving").rect().center();
    let bar_rect = h
        .ctx
        .memory(|m| m.area_rect(egui::Id::new("command-bar")))
        .expect("command bar is laid out");
    assert!(
        bar_rect.contains(quit_btn),
        "test setup must overlap command bar ({bar_rect:?}) and Quit button ({quit_btn:?})"
    );
    h.event(egui::Event::PointerMoved(quit_btn));
    h.step();
    crate::ui_tests::press(&h, quit_btn, egui::PointerButton::Primary, true);
    h.step();
    crate::ui_tests::press(&h, quit_btn, egui::PointerButton::Primary, false);
    h.run_steps(2);
    assert!(
        h.state().quit_confirmed,
        "unsaved-edits Quit button must be clickable above an open Command Bar"
    );
    assert_eq!(h.state().unsaved, None);
}

#[test]
fn unsaved_modal_stays_above_space_popover_and_quit_button_is_clickable() {
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(540.0, 380.0))
        .build_eframe(|cc| {
            let mut app =
                crate::App::new(cc, vec![crate::ui_tests::fixture(crate::ui_tests::FIXTURE)]);
            app.show_left = false;
            app.show_right = false;
            app
        });
    crate::ui_tests::wait(&mut h, &mut Vec::new(), "the file to load", |a| {
        a.tabs
            .first()
            .is_some_and(|t| t.canvas_rect.is_some() && !t.fit_pending)
    });
    let ctx = h.ctx.clone();
    h.state_mut().tabs[0].select(Some(Guid(1, 25)));
    h.state_mut().apply(Action::DuplicateSelection, &ctx);
    assert!(h.state().tabs[0].dirty_unsaved);

    // Show and cancel the unsaved modal once first, then open a fresh Space Popover and Quit.
    h.state_mut().apply(Action::CloseTab, &ctx);
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert_eq!(h.state().unsaved, None);

    h.state_mut().apply(Action::SpaceColour, &ctx);
    h.run_steps(3);
    assert!(h.state().space_editing.is_some());

    h.state_mut().apply(Action::Quit, &ctx);
    h.run_steps(3);
    assert_eq!(h.state().unsaved, Some(Pending::Quit));

    let quit_btn = h.get_by_label("Quit without saving").rect().center();
    h.event(egui::Event::PointerMoved(quit_btn));
    h.step();
    crate::ui_tests::press(&h, quit_btn, egui::PointerButton::Primary, true);
    h.step();
    crate::ui_tests::press(&h, quit_btn, egui::PointerButton::Primary, false);
    h.run_steps(2);
    assert!(
        h.state().quit_confirmed,
        "unsaved-edits Quit button must be clickable above an open Space Popover"
    );
    assert_eq!(h.state().unsaved, None);
}

#[test]
fn unsaved_modal_blocks_peek_sidebar_background_text_edit_and_welcome_keys() {
    // 1. peek_sidebar triggered at the left edge must not cover or block unsaved-edits.
    {
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(540.0, 380.0))
            .build_eframe(|cc| {
                let mut app =
                    crate::App::new(cc, vec![crate::ui_tests::fixture(crate::ui_tests::FIXTURE)]);
                app.show_left = false;
                app.show_right = false;
                app
            });
        crate::ui_tests::wait(&mut h, &mut Vec::new(), "the file to load", |a| {
            a.tabs
                .first()
                .is_some_and(|t| t.canvas_rect.is_some() && !t.fit_pending)
        });
        let ctx = h.ctx.clone();
        h.state_mut().tabs[0].select(Some(Guid(1, 25)));
        h.state_mut().apply(Action::DuplicateSelection, &ctx);
        // Open and cancel once first so unsaved-edits is already in egui's order list.
        h.state_mut().apply(Action::CloseTab, &ctx);
        h.run_steps(2);
        h.get_by_label("Cancel").click();
        h.run_steps(2);

        h.state_mut().apply(Action::Quit, &ctx);
        h.run_steps(2);
        // Move pointer to left edge to trigger peek_sidebar while unsaved-edits is open,
        // then immediately click Cancel where peek_sidebar overlaps it.
        h.event(egui::Event::PointerMoved(egui::pos2(2.0, 180.0)));
        h.step();
        let cancel_pt = h.get_by_label("Cancel").rect().center();
        h.event(egui::Event::PointerMoved(cancel_pt));
        h.step();
        crate::ui_tests::press(&h, cancel_pt, egui::PointerButton::Primary, true);
        h.step();
        crate::ui_tests::press(&h, cancel_pt, egui::PointerButton::Primary, false);
        h.run_steps(2);
        assert_eq!(
            h.state().unsaved,
            None,
            "peek_sidebar must not intercept Cancel clicks on unsaved-edits"
        );
    }

    // 2. Previously focused Edit Layer TextEdit must not accept typing on the first frame
    // or subsequent frames while unsaved-edits is up.
    {
        let mut h = crate::ui_tests::harness();
        let ctx = h.ctx.clone();
        let target = Guid(1, 34);
        h.state_mut().tabs[0].select(Some(target));
        h.run_steps(3);
        h.get_by_label("Edit Layer").click();
        h.run_steps(3);
        h.get_by_label("Layer name").click();
        h.step();
        h.get_by_label("Layer name").type_text("A");
        h.run_steps(2);
        assert!(h.state().tabs[0].dirty_unsaved);
        let name_before = h.state().tabs[0].doc.node(target).unwrap().name.clone();
        let rev_before = h.state().tabs[0].doc.revision;

        h.state_mut().apply(Action::Quit, &ctx);
        // Send text on the very first frame unsaved-edits renders (before egui swaps top_modal_layer).
        h.event(egui::Event::Text("Z".into()));
        h.run_steps(2);
        assert_eq!(
            h.state().tabs[0].doc.node(target).unwrap().name,
            name_before,
            "typing behind unsaved-edits must not edit the previously focused Layer name field"
        );
        assert_eq!(h.state().tabs[0].doc.revision, rev_before);
    }

    // 3. Welcome tour keys (ArrowRight / Enter) must not advance behind unsaved-edits.
    {
        let mut h = edited();
        let ctx = h.ctx.clone();
        h.state_mut().apply(Action::Welcome, &ctx);
        h.run_steps(3);
        assert!(h.state().onboarding.sheet_open());
        assert!(
            h.query_by_label("Skip").is_some(),
            "welcome starts on page 0"
        );

        h.state_mut().apply(Action::Quit, &ctx);
        h.run_steps(3);
        assert_eq!(h.state().unsaved, Some(Pending::Quit));

        h.key_press(egui::Key::ArrowRight);
        h.run_steps(2);
        assert!(
            h.query_by_label("Skip").is_some() && h.query_by_label("Back").is_none(),
            "ArrowRight behind unsaved-edits must not advance the welcome sheet"
        );
        assert!(h.state().onboarding.sheet_open());
    }
}

#[test]
fn async_save_copy_completion_handles_success_cancel_failure_and_keeps_normal_save_dirty() {
    use crate::unsaved::{SaveOutcome, SaveSnapshot};

    // Normal Save a copy stays a copy across both Saved (picker) and Downloaded (Blob fallback).
    for outcome in [
        SaveOutcome::Saved("copy.fig".into()),
        SaveOutcome::Downloaded("copy.fig".into()),
    ] {
        let mut h = edited();
        let ctx = h.ctx.clone();
        let snap = SaveSnapshot::of(&h.state().tabs[0]);
        let save_id = h
            .state_mut()
            .begin_async_save_copy(snap, false)
            .expect("starts normal async save");
        h.state_mut()
            .complete_async_save_copy(&ctx, save_id, outcome);
        assert_eq!(
            h.state().tabs.len(),
            1,
            "normal Save a copy must not close the tab"
        );
        assert!(
            h.state().tabs[0].dirty_unsaved,
            "normal Save a copy must keep the tab unsaved"
        );
        assert!(!h.state().toasts.iter().any(|t| t.error));
    }

    // Cancelled and failed close-triggered saves keep the tab open and unsaved.
    let mut h = edited();
    let ctx = h.ctx.clone();
    let snap = SaveSnapshot::of(&h.state().tabs[0]);

    let cancel_id = h
        .state_mut()
        .begin_async_save_copy(snap, true)
        .expect("starts close-triggered save");
    // A mismatched/stale save id must be ignored without clearing the active save.
    h.state_mut().complete_async_save_copy(
        &ctx,
        cancel_id + 99,
        SaveOutcome::Saved("stale.fig".into()),
    );
    assert_eq!(h.state().tabs.len(), 1);
    assert!(h.state().save_in_flight.is_some());

    h.state_mut()
        .complete_async_save_copy(&ctx, cancel_id, SaveOutcome::Cancelled);
    assert_eq!(h.state().tabs.len(), 1, "cancelled save keeps tab open");
    assert!(h.state().tabs[0].dirty_unsaved);
    assert!(!h.state().toasts.iter().any(|t| t.error));

    let fail_id = h
        .state_mut()
        .begin_async_save_copy(snap, true)
        .expect("starts retry after cancel");
    h.state_mut()
        .complete_async_save_copy(&ctx, fail_id, SaveOutcome::Err("disk full".into()));
    assert_eq!(h.state().tabs.len(), 1, "failed save keeps tab open");
    assert!(h.state().tabs[0].dirty_unsaved);
    assert!(
        h.state()
            .toasts
            .iter()
            .any(|t| t.error && t.text.contains("disk full")),
        "failed save must surface an error toast"
    );

    // Matching confirmed close-triggered save (`Saved`) closes the tab.
    {
        let mut h = edited();
        let ctx = h.ctx.clone();
        let snap = SaveSnapshot::of(&h.state().tabs[0]);
        let save_id = h
            .state_mut()
            .begin_async_save_copy(snap, true)
            .expect("starts close-triggered save");
        h.state_mut().complete_async_save_copy(
            &ctx,
            save_id,
            SaveOutcome::Saved("close-copy.fig".into()),
        );
        assert!(
            h.state().tabs.is_empty(),
            "confirmed close-save must close the tab"
        );
        assert!(!h.state().any_unsaved());
    }

    // An unconfirmed browser download (`Downloaded`) must keep the dirty tab open and retain its
    // edited document content instead of discarding it on close.
    {
        let mut h = edited();
        let ctx = h.ctx.clone();
        let snap = SaveSnapshot::of(&h.state().tabs[0]);
        let edited_nodes = h.state().tabs[0].doc.nodes.len();
        let edited_rev = h.state().tabs[0].doc.revision;
        let save_id = h
            .state_mut()
            .begin_async_save_copy(snap, true)
            .expect("starts close-triggered download");
        h.state_mut().complete_async_save_copy(
            &ctx,
            save_id,
            SaveOutcome::Downloaded("close-copy.fig".into()),
        );
        assert_eq!(
            h.state().tabs.len(),
            1,
            "unconfirmed download must not auto-close a dirty tab"
        );
        assert!(h.state().tabs[0].dirty_unsaved);
        assert!(h.state().any_unsaved());
        assert_eq!(h.state().tabs[0].doc.nodes.len(), edited_nodes);
        assert_eq!(h.state().tabs[0].doc.revision, edited_rev);
        assert!(
            h.state().toasts.iter().any(|t| !t.error
                && t.text.contains("Started download")
                && t.text.contains("stays open")),
            "toast must report an unconfirmed download and that the tab stays open"
        );
    }
}

#[test]
fn concurrent_edit_during_in_flight_save_never_discards_newer_edits() {
    use crate::unsaved::{SaveOutcome, SaveSnapshot};

    let mut h = edited();
    let ctx = h.ctx.clone();
    let snap_before = SaveSnapshot::of(&h.state().tabs[0]);
    let save_id = h
        .state_mut()
        .begin_async_save_copy(snap_before, true)
        .expect("starts close-triggered save");

    // Make a concurrent edit while the save is still in flight.
    h.state_mut().apply(Action::DuplicateSelection, &ctx);
    let nodes_after_concurrent = h.state().tabs[0].doc.nodes.len();
    assert_ne!(SaveSnapshot::of(&h.state().tabs[0]), snap_before);

    // When the older save finishes, the newer edit must NOT be discarded by closing the tab.
    h.state_mut().complete_async_save_copy(
        &ctx,
        save_id,
        SaveOutcome::Saved("older-snapshot.fig".into()),
    );
    assert_eq!(
        h.state().tabs.len(),
        1,
        "concurrent edit must keep the tab open when an older save finishes"
    );
    assert_eq!(h.state().tabs[0].doc.nodes.len(), nodes_after_concurrent);
    assert!(h.state().tabs[0].dirty_unsaved);
}

#[test]
fn save_in_flight_close_merges_matching_intent_and_rejects_stale_or_discarded_tab() {
    use crate::unsaved::{SaveOutcome, SaveSnapshot};

    // 1. Normal Save a copy in flight for S1 -> user clicks Close -> "Save a copy…" on S1 ->
    //    close intent merges onto the matching save and closes the tab on completion.
    {
        let mut h = edited();
        let ctx = h.ctx.clone();
        let snap = SaveSnapshot::of(&h.state().tabs[0]);
        let save_id = h
            .state_mut()
            .begin_async_save_copy(snap, false)
            .expect("normal save in flight");

        h.state_mut().apply(Action::CloseTab, &ctx);
        h.run_steps(2);
        h.get_by_label("Save a copy…").click();
        h.run_steps(2);
        assert_eq!(h.state().tabs.len(), 1, "tab waits until save succeeds");
        assert!(
            !h.state().toasts.iter().any(|t| t.error),
            "matching in-flight save must not error when close asks to save a copy"
        );

        h.state_mut().complete_async_save_copy(
            &ctx,
            save_id,
            SaveOutcome::Saved("merged-close.fig".into()),
        );
        assert!(
            h.state().tabs.is_empty(),
            "merged close intent must close the tab after matching save succeeds"
        );
    }

    // 2. Close-triggered save in flight -> normal SaveCopy must NOT suppress close intent;
    //    a duplicate Close modal left open is cleared when the confirmed save closes the tab.
    {
        let mut h = edited();
        let ctx = h.ctx.clone();
        let snap = SaveSnapshot::of(&h.state().tabs[0]);
        let save_id = h
            .state_mut()
            .begin_async_save_copy(snap, true)
            .expect("close-triggered save in flight");

        h.state_mut().apply(Action::SaveCopy, &ctx);
        h.state_mut().apply(Action::CloseTab, &ctx);
        assert_eq!(h.state().unsaved, Some(Pending::Tab(snap.tab)));

        h.state_mut().complete_async_save_copy(
            &ctx,
            save_id,
            SaveOutcome::Saved("close-copy.fig".into()),
        );
        assert!(
            h.state().tabs.is_empty(),
            "normal SaveCopy while close-save is in flight must not suppress valid close intent"
        );
        assert_eq!(
            h.state().unsaved,
            None,
            "closing the tab on confirmed save completion must clear its pending close modal"
        );
    }

    // 2b. Normal save in flight upgraded by Close -> "Save a copy…" that falls back to an
    //     unconfirmed download keeps the dirty tab open and preserves edits.
    {
        let mut h = edited();
        let ctx = h.ctx.clone();
        let snap = SaveSnapshot::of(&h.state().tabs[0]);
        let edited_nodes = h.state().tabs[0].doc.nodes.len();
        let save_id = h
            .state_mut()
            .begin_async_save_copy(snap, false)
            .expect("normal save in flight");

        h.state_mut().apply(Action::CloseTab, &ctx);
        h.run_steps(2);
        h.get_by_label("Save a copy…").click();
        h.run_steps(2);

        h.state_mut().complete_async_save_copy(
            &ctx,
            save_id,
            SaveOutcome::Downloaded("merged-download.fig".into()),
        );
        assert_eq!(
            h.state().tabs.len(),
            1,
            "merged close intent must not close tab on unconfirmed download"
        );
        assert!(h.state().tabs[0].dirty_unsaved);
        assert_eq!(h.state().tabs[0].doc.nodes.len(), edited_nodes);
    }

    // 3. Normal save in flight for S1 -> user edits to S2 -> Close -> "Save a copy…" rejects
    //    as busy and completing the stale S1 save keeps S2 open.
    {
        let mut h = edited();
        let ctx = h.ctx.clone();
        let snap_s1 = SaveSnapshot::of(&h.state().tabs[0]);
        let save_id = h
            .state_mut()
            .begin_async_save_copy(snap_s1, false)
            .expect("normal save S1 in flight");

        h.state_mut().apply(Action::DuplicateSelection, &ctx);
        let nodes_s2 = h.state().tabs[0].doc.nodes.len();

        h.state_mut().apply(Action::CloseTab, &ctx);
        h.run_steps(2);
        h.get_by_label("Save a copy…").click();
        h.run_steps(2);
        assert!(
            h.state()
                .toasts
                .iter()
                .any(|t| t.error && t.text.contains("already running")),
            "close-save on newer state while an older save runs must report busy"
        );

        h.state_mut().complete_async_save_copy(
            &ctx,
            save_id,
            SaveOutcome::Saved("stale-s1.fig".into()),
        );
        assert_eq!(h.state().tabs.len(), 1);
        assert_eq!(h.state().tabs[0].doc.nodes.len(), nodes_s2);
        assert!(h.state().tabs[0].dirty_unsaved);
    }

    // 4. Discarding a tab while its close-save is in flight closes only that tab; when the
    //    in-flight save finishes later, another open tab stays untouched.
    {
        let mut h = edited();
        let ctx = h.ctx.clone();
        let doc2 = h.state().tabs[0].doc.clone();
        let tab2_id = h.state().next_id;
        h.state_mut().next_id += 1;
        let tab2 = crate::tab::Tab::new(
            tab2_id,
            crate::ui_tests::fixture("vector.fig"),
            doc2,
            None,
            std::time::Duration::ZERO,
        );
        h.state_mut().tabs.push(tab2);
        h.state_mut().active = 0;

        let snap0 = SaveSnapshot::of(&h.state().tabs[0]);
        let save_id = h
            .state_mut()
            .begin_async_save_copy(snap0, true)
            .expect("close-save on tab 0");

        h.state_mut().apply(Action::CloseTab, &ctx);
        h.run_steps(2);
        h.get_by_label("Discard edits").click();
        h.run_steps(2);
        assert_eq!(h.state().tabs.len(), 1);
        assert_eq!(h.state().tabs[0].id, tab2_id);

        h.state_mut().complete_async_save_copy(
            &ctx,
            save_id,
            SaveOutcome::Saved("discarded-tab-copy.fig".into()),
        );
        assert_eq!(h.state().tabs.len(), 1, "other open tab must stay open");
        assert_eq!(h.state().tabs[0].id, tab2_id);
    }
}

#[test]
fn browser_destination_identity_fails_closed_for_same_and_unknown() {
    use crate::unsaved::{DestinationIdentity, HandleSaveDecision};

    for (tag, expected_id, expected_decision) in [
        (
            Some("same"),
            DestinationIdentity::Same,
            HandleSaveDecision::RejectSameSource,
        ),
        (
            Some("different"),
            DestinationIdentity::Different,
            HandleSaveDecision::WriteHandle,
        ),
        (
            Some("unknown"),
            DestinationIdentity::Unknown,
            HandleSaveDecision::FallbackDownload,
        ),
        (
            None,
            DestinationIdentity::Unknown,
            HandleSaveDecision::FallbackDownload,
        ),
        (
            Some(""),
            DestinationIdentity::Unknown,
            HandleSaveDecision::FallbackDownload,
        ),
        (
            Some("false"),
            DestinationIdentity::Unknown,
            HandleSaveDecision::FallbackDownload,
        ),
    ] {
        let id = DestinationIdentity::from_js_tag(tag);
        assert_eq!(id, expected_id, "tag={tag:?}");
        assert_eq!(id.decide(), expected_decision, "tag={tag:?}");
    }
}

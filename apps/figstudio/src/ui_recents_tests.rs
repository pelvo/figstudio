//! The Recents page through the real app (`recents.rs`). The list lives in memory here.
use egui::accesskit::Role;
use egui_kittest::kittest::NodeT as _;

use super::*;

/// The app with nothing open and `files` opened before, newest first.
fn with_recents(files: &[PathBuf]) -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, Vec::new()));
    let now = crate::recents::now();
    for (i, path) in files.iter().rev().enumerate() {
        h.state_mut()
            .recents
            .note(path, None, now - 3600 + i as u64);
    }
    h.run_steps(3);
    h
}

fn has(h: &Harness<'_, App>, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

/// Steps until a node is labelled `label` (the page looks at the disk on another thread,
/// [`crate::recents`]'s `RECHECK` after its last look returned; a step is 0.25 s of egui
/// time).
fn wait_for(h: &mut Harness<'_, App>, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !has(h, label) {
        assert!(Instant::now() < deadline, "timed out waiting for {label:?}");
        std::thread::sleep(Duration::from_millis(5));
        h.step();
    }
}

#[test]
fn with_nothing_opened_before_the_page_welcomes_and_offers_open_file() {
    let h = with_recents(&[]);
    assert!(has(&h, "Recents"));
    // The page's own button, with its keys (the sidebar's + is "Open file…" too).
    assert!(h.query_by_label_contains("Open file… (").is_some());
    assert!(!has(&h, "Grid view"), "no view toggle over an empty page");
}

#[test]
fn the_welcome_waits_for_the_saved_list() {
    let mut h = with_recents(&[]);
    // The browser reads its list a few frames after the page first shows.
    h.state_mut().recents.waiting = true;
    h.run_steps(2);
    assert!(has(&h, "Recents"));
    assert!(!has(&h, "Files you open show up here."));
    assert!(h.query_by_label_contains("Open file… (").is_none());
    h.state_mut().recents.take(crate::recents::News::default());
    h.run_steps(2);
    assert!(
        !has(&h, "Files you open show up here."),
        "a look is not the list"
    );
    h.state_mut().recents.take(crate::recents::News {
        entries: Some(Vec::new()),
        ..Default::default()
    });
    h.run_steps(2);
    assert!(has(&h, "Files you open show up here."));
}

#[test]
fn a_file_opened_once_comes_back_as_a_card_that_opens_it() {
    let mut h = harness();
    let path = std::path::absolute(fixture(FIXTURE)).unwrap();
    assert_eq!(
        h.state().recents.entries().first().map(|e| &e.path),
        Some(&path),
        "loading a file lists it"
    );
    h.state_mut().close(0);
    h.run_steps(3);
    assert!(has(&h, "Recents"));
    h.get_by_label(FIXTURE).click();
    wait(&mut h, &mut Vec::new(), "the file to open again", |a| {
        a.tabs.len() == 1
    });
    assert!(!has(&h, "Recents"), "an open file replaces the page");
}

#[test]
fn the_filter_narrows_the_cards_and_says_when_none_match() {
    let mut h = with_recents(&[fixture(FIXTURE), fixture("sections.fig")]);
    assert!(has(&h, FIXTURE) && has(&h, "sections.fig"));
    let field =
        h.get_by(|n| n.role() == Role::TextInput && n.placeholder() == Some("Filter recents"));
    field.focus();
    field.type_text("SECT");
    h.run_steps(3);
    assert!(has(&h, "sections.fig"));
    assert!(!has(&h, FIXTURE), "case does not matter, other files go");
    // A field with text has no placeholder: found by what it holds.
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("SECT"))
        .type_text("zzz");
    h.run_steps(3);
    assert!(has(&h, "No recent files match"));
}

#[test]
fn the_view_toggle_switches_to_rows_and_is_kept_in_prefs() {
    let mut h = with_recents(&[fixture(FIXTURE), fixture("sections.fig")]);
    let card = h.get_by_label(FIXTURE).rect();
    h.get_by_label("List view").click();
    h.run_steps(3);
    assert!(h.state().prefs.recents_list);
    let row = h.get_by_label(FIXTURE).rect();
    assert!(
        row.width() > card.width() * 2.0 && row.height() < card.height() / 2.0,
        "a row spans the page: card {card:?}, row {row:?}"
    );
    h.get_by_label("Grid view").click();
    h.run_steps(3);
    assert!(!h.state().prefs.recents_list);
}

#[test]
fn remove_from_recents_takes_the_card_away() {
    let mut h = with_recents(&[fixture(FIXTURE), fixture("sections.fig")]);
    h.get_by_label(FIXTURE).click_secondary();
    h.run_steps(2);
    h.get_by_label("Remove from recents").click();
    h.run_steps(3);
    assert!(!has(&h, FIXTURE));
    assert!(has(&h, "sections.fig"));
    assert_eq!(h.state().recents.entries().len(), 1);
    assert!(h.state().tabs.is_empty(), "removing does not open it");
}

#[test]
fn a_moved_file_says_so_and_does_not_open() {
    let gone = std::env::temp_dir().join("figstudio-recents-test-gone/old.fig");
    let mut h = with_recents(&[gone]);
    let card = h.get_by_label("old.fig (missing)");
    card.click();
    h.run_steps(3);
    assert!(h.state().tabs.is_empty() && h.state().loading.len() == 0);
    assert!(
        h.state()
            .toasts
            .iter()
            .any(|t| t.error && t.text.contains("moved or deleted")),
        "a toast says why"
    );
    h.get_by_label("old.fig (missing)").click_secondary();
    h.run_steps(2);
    h.get_by_label("Remove from recents").click();
    h.run_steps(3);
    assert!(h.state().recents.entries().is_empty());
}

#[test]
fn the_window_is_slate_and_takes_the_colour_of_the_card_under_the_pointer() {
    let mut h = with_recents(&[fixture(FIXTURE)]);
    let dark = crate::theme::palette(&h.ctx).dark;
    let tint = |h: &Harness<'_, App>| h.state().tint_now(&h.ctx);
    let slate = crate::theme::tint(7, dark);
    assert_eq!(crate::theme::TINT_NAMES[crate::theme::HOME_TINT], "Slate");
    assert_eq!(tint(&h).accent, slate.accent, "no file open: slate");
    let card = crate::theme::space_tint(
        crate::theme::Colour::Preset(crate::theme::pick_tint(FIXTURE, [])),
        dark,
    );
    assert_ne!(
        card.accent, slate.accent,
        "the test needs a card of another colour"
    );
    h.get_by_label(FIXTURE).hover();
    h.run_steps(3);
    assert_eq!(
        tint(&h).accent,
        card.accent,
        "the hovered card's space colour"
    );
    h.event(egui::Event::PointerMoved(egui::pos2(5.0, SIZE.y - 5.0)));
    h.run_steps(3);
    assert_eq!(tint(&h).accent, slate.accent, "back to slate off the card");
}

/// A copy of the fixture in its own temporary folder (deleted with the folder).
fn temp_copy(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::copy(fixture(FIXTURE), &path).unwrap();
    path
}

#[test]
fn a_file_dropped_as_bytes_without_a_real_path_is_not_listed() {
    let mut h = with_recents(&[]);
    let bytes = std::fs::read(fixture(FIXTURE)).unwrap();
    h.state()
        .workers
        .load_bytes(PathBuf::from("dropped-without-a-path.fig"), bytes);
    wait(&mut h, &mut Vec::new(), "the dropped file", |a| {
        a.tabs.len() == 1
    });
    assert!(
        h.state().recents.entries().is_empty(),
        "a card for it could never open"
    );
}

#[test]
fn a_file_deleted_and_put_back_while_the_page_shows_opens_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = temp_copy(&dir, "kept.fig");
    let mut h = with_recents(std::slice::from_ref(&path));
    std::fs::remove_file(&path).unwrap();
    h.get_by_label("kept.fig").click();
    h.run_steps(3);
    assert!(h.state().tabs.is_empty());
    assert!(has(&h, "kept.fig (missing)"), "the card now says so");
    temp_copy(&dir, "kept.fig");
    h.get_by_label("kept.fig (missing)").click();
    wait(
        &mut h,
        &mut Vec::new(),
        "the file back in place to open",
        |a| a.tabs.len() == 1,
    );
}

#[test]
fn a_narrow_window_keeps_the_header_inside_the_card() {
    let mut h = Harness::builder()
        .with_size(egui::vec2(560.0, 600.0))
        .build_eframe(|cc| App::new(cc, Vec::new()));
    let now = crate::recents::now();
    h.state_mut().recents.note(&fixture(FIXTURE), None, now);
    h.run_steps(3);
    let card = h.state().card_rect.expect("the card is drawn");
    let title = h.get_by_label("Recents").rect();
    let open = h.get_by_label_contains("Open file… (").rect();
    assert!(
        card.contains_rect(open),
        "Open file… inside the card: {open:?} in {card:?}"
    );
    assert!(
        open.left() > title.right(),
        "clear of the title: {open:?}, {title:?}"
    );
    for control in ["Grid view", "List view"] {
        if let Some(n) = h.query_by_label(control) {
            let r = n.rect();
            assert!(
                card.contains_rect(r) && r.left() > title.right(),
                "{control}: {r:?}"
            );
        }
    }
}

#[test]
fn a_file_put_back_while_the_page_shows_is_offered_again_in_the_menu() {
    let dir = tempfile::tempdir().unwrap();
    let path = temp_copy(&dir, "back.fig");
    let mut h = with_recents(std::slice::from_ref(&path));
    std::fs::remove_file(&path).unwrap();
    // The page sees it gone (the next time it looks), then back.
    wait_for(&mut h, "back.fig (missing)");
    temp_copy(&dir, "back.fig");
    wait_for(&mut h, "back.fig");
    h.get_by_label("back.fig").click_secondary();
    h.run_steps(3);
    let open = h.get_by_label("Open");
    assert!(
        !open.accesskit_node().is_disabled(),
        "the menu opens it again"
    );
}

/// The app on `structure.fig` and `vector.fig`, the second one shown.
fn two_files() -> Harness<'static, App> {
    let mut h = harness();
    h.state_mut().open(fixture("vector.fig"));
    wait(&mut h, &mut Vec::new(), "the second file", |a| {
        a.tabs.len() == 2 && a.active == 1
    });
    h
}

fn on_recents(app: &App) -> bool {
    app.tabs.get(app.active).is_none()
}

#[test]
fn the_plus_button_shows_recents_with_the_files_still_open() {
    let mut h = harness();
    assert!(!has(&h, "Recents"));
    h.get_by_label("Open file…").click();
    h.run_steps(3);
    assert!(has(&h, "Recents"), "the + shows the Recents page");
    assert_eq!(h.state().tabs.len(), 1, "the file stays open");
    assert!(!has(&h, "No file open"), "the sidebar does not say none is");
    // The card of the open file (not its space's dot in the sidebar) goes back to it.
    h.get_all_by_label(FIXTURE)
        .max_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
        .expect("the card")
        .click();
    wait(&mut h, &mut Vec::new(), "the file to show", |a| {
        !on_recents(a)
    });
    assert_eq!(h.state().tabs.len(), 1, "no second copy");
    h.run_steps(2);
    assert!(!has(&h, "Recents"));
}

#[test]
fn the_recents_page_comes_after_the_last_space_and_stays_when_one_closes() {
    let mut h = two_files();
    let ctx = h.ctx.clone();
    let from_recents = |h: &mut Harness<'static, App>, action| {
        h.get_by_label("Open file…").click();
        h.run_steps(3);
        h.state_mut().apply(action, &ctx);
        h.run_steps(3);
        h.state().active
    };
    assert_eq!(from_recents(&mut h, crate::menu::Action::NextSpace), 0);
    assert_eq!(from_recents(&mut h, crate::menu::Action::PreviousSpace), 1);
    h.get_by_label("Open file…").click();
    h.run_steps(3);
    h.state_mut().close(0);
    h.run_steps(3);
    assert!(on_recents(h.state()), "closing a space keeps the page");
    assert!(has(&h, "Recents"));
}

#[test]
fn the_window_title_is_the_app_on_the_recents_page() {
    let mut h = harness();
    h.get_by_label("Open file…").click();
    h.run_steps(3);
    assert_eq!(h.state().window_title, crate::APP_NAME);
}

#[test]
fn a_deleted_file_picked_in_the_dock_menu_says_so_and_does_not_open() {
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("dock.fig");
    std::fs::copy(fixture(FIXTURE), &gone).unwrap();
    let mut h = with_recents(std::slice::from_ref(&gone));
    assert_eq!(h.state().recents.shortlist(), std::slice::from_ref(&gone));
    // Deleted after the menu was made.
    std::fs::remove_file(&gone).unwrap();
    let ctx = h.ctx.clone();
    h.state_mut().open_recent_file(&ctx, gone.clone());
    h.run_steps(3);
    assert!(h.state().tabs.is_empty() && h.state().loading.len() == 0);
    assert!(
        h.state()
            .toasts
            .iter()
            .any(|t| t.error && t.text.contains("moved or deleted")),
        "a toast says why"
    );
    assert!(
        h.state().recents.shortlist().is_empty(),
        "it leaves the dock menu"
    );
}

#[test]
fn a_file_picked_in_the_dock_menu_opens() {
    let path = std::path::absolute(fixture(FIXTURE)).unwrap();
    let mut h = with_recents(std::slice::from_ref(&path));
    let ctx = h.ctx.clone();
    h.state_mut().open_recent_file(&ctx, path);
    wait(&mut h, &mut Vec::new(), "the file to open", |a| {
        a.tabs.len() == 1
    });
}

#[test]
fn a_listed_file_gone_when_opened_at_start_is_marked_missing() {
    // A Windows jump list item starts figstudio with its path; the file may be gone.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("went.fig");
    std::fs::copy(fixture(FIXTURE), &path).unwrap();
    let mut h = with_recents(std::slice::from_ref(&path));
    std::fs::remove_file(&path).unwrap();
    h.state_mut().open(path.clone());
    wait(&mut h, &mut Vec::new(), "the open to fail", |a| {
        a.loading.len() == 0 && !a.toasts.is_empty()
    });
    assert!(h.state().tabs.is_empty());
    assert!(
        h.state()
            .recents
            .is_missing(&crate::recents::Id::file(path)),
        "marked missing"
    );
    assert!(
        h.state().recents.shortlist().is_empty(),
        "it leaves the jump list"
    );
}

#[test]
fn the_workspace_saved_on_the_recents_page_keeps_the_file_it_was_on() {
    let mut h = two_files();
    let shown = h.state().tabs[1].path.clone();
    h.get_by_label("Open file…").click();
    h.run_steps(3);
    assert!(on_recents(h.state()));
    assert_eq!(
        crate::session::Session::capture(h.state()).active(),
        Some(std::path::absolute(shown).unwrap().as_path()),
        "the next launch shows the file the + was pressed on"
    );
}

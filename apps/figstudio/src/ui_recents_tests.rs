//! The Recents page through the real app (`recents.rs`). The list lives in memory here.
use egui::accesskit::Role;

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

#[test]
fn with_nothing_opened_before_the_page_welcomes_and_offers_open_file() {
    let h = with_recents(&[]);
    assert!(has(&h, "Recents"));
    // The page's own button, with its keys (the sidebar's + is "Open file…" too).
    assert!(h.query_by_label_contains("Open file… (").is_some());
    assert!(!has(&h, "Grid view"), "no view toggle over an empty page");
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

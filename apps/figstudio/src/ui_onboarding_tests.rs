//! The welcome sheet and the one-time tips through the real app (`onboarding.rs`).
use egui::{Key, Modifiers};

use super::*;
use crate::onboarding::{Onboarding, Progress};

/// Steps until `label` is on screen (or gone, with `shown` false), at most 10 s. A new card's
/// first frame is egui's sizing pass, laid out where it will not stay: a few more frames put
/// it in place before anything is clicked.
fn wait_label(h: &mut Harness<'_, App>, label: &str, shown: bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.query_by_label(label).is_some() != shown {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {label:?} to be {}",
            if shown { "shown" } else { "gone" }
        );
        h.step();
    }
    if shown {
        h.run_steps(3);
    }
}

/// The app with nothing open, as on a first launch with onboarding on.
fn first_launch() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, Vec::new()));
    h.state_mut().onboarding = Onboarding::new(Progress::default(), true);
    h.run_steps(2);
    h
}

/// `structure.fig` open, the welcome already seen, no tips yet.
fn welcomed() -> Harness<'static, App> {
    let mut h = harness();
    h.state_mut().onboarding = Onboarding::new(
        Progress {
            welcomed: true,
            ..Progress::default()
        },
        true,
    );
    h
}

#[test]
fn the_welcome_sheet_walks_through_its_pages_and_stays_finished() {
    let mut h = first_launch();
    wait_label(&mut h, "Welcome to figstudio", true);
    h.get_by_label("Continue").click();
    wait_label(&mut h, "Every file is a space", true);
    assert!(h.query_by_label("Welcome to figstudio").is_none());
    h.get_by_label("Back").click();
    wait_label(&mut h, "Welcome to figstudio", true);
    // The keys move through it too.
    h.key_press(Key::ArrowRight);
    wait_label(&mut h, "Every file is a space", true);
    h.key_press(Key::Enter);
    wait_label(&mut h, "Find anything", true);
    assert!(
        h.query_by_label("Continue").is_none(),
        "the last page ends it"
    );
    h.get_by_label("Get started").click();
    wait_label(&mut h, "Find anything", false);
    let o = &h.state().onboarding;
    assert!(o.progress.welcomed && !o.sheet_open());
    assert!(!h.state().onboarding.sheet_up(&h.ctx), "gone, not fading");
}

#[test]
fn the_sheet_is_modal_and_escape_skips_it() {
    let mut h = first_launch();
    wait_label(&mut h, "Welcome to figstudio", true);
    h.key_press_modifiers(Modifiers::COMMAND, Key::L);
    h.run_steps(3);
    assert!(
        !h.state().command.is_open(),
        "shortcuts wait while the sheet is up"
    );
    h.key_press(Key::Escape);
    wait_label(&mut h, "Welcome to figstudio", false);
    assert!(h.state().onboarding.progress.welcomed);
    h.key_press_modifiers(Modifiers::COMMAND, Key::L);
    h.run_steps(3);
    assert!(h.state().command.is_open(), "and work again after");
}

#[test]
fn tips_come_one_at_a_time_beside_their_control_and_go_when_it_is_used() {
    let mut h = welcomed();
    wait_label(&mut h, "Search everything", true);
    assert!(
        h.query_by_label("One space per file").is_none(),
        "one at a time"
    );
    // The card sits to the right of the command field, level with it.
    let field = h.state().onboarding.targets.command.expect("drawn");
    let card = h.get_by_label("Search everything").rect();
    assert!(card.left() > field.right(), "{card:?} beside {field:?}");
    assert!(
        card.top() < field.bottom() && card.bottom() > field.top(),
        "level with it"
    );

    // Using the command bar is learning it.
    h.key_press_modifiers(Modifiers::COMMAND, Key::L);
    h.run_steps(2);
    assert!(h.state().onboarding.progress.tip_command);
    h.key_press(Key::Escape);
    wait_label(&mut h, "Search everything", false);
    wait_label(&mut h, "One space per file", true);

    h.get_by_label("Got it").click();
    wait_label(&mut h, "One space per file", false);
    assert!(h.state().onboarding.progress.tip_spaces);
    wait_label(&mut h, "Make room", true);
    // Hiding the sidebar is the last one.
    h.get_by_label("Hide sidebar").click();
    wait_label(&mut h, "Make room", false);
    assert_eq!(
        h.state().onboarding.progress,
        Progress {
            welcomed: true,
            tip_command: true,
            tip_spaces: true,
            tip_sidebar: true,
        }
    );
}

#[test]
fn skip_tips_ends_them_all() {
    let mut h = welcomed();
    wait_label(&mut h, "Search everything", true);
    h.get_by_label("Skip tips").click();
    wait_label(&mut h, "Search everything", false);
    h.run_steps(10);
    for title in ["One space per file", "Make room"] {
        assert!(h.query_by_label(title).is_none(), "{title}");
    }
    let p = &h.state().onboarding.progress;
    assert!(p.tip_command && p.tip_spaces && p.tip_sidebar);
}

#[test]
fn help_welcome_tour_from_the_command_bar_shows_it_all_again() {
    let mut h = harness();
    h.state_mut().onboarding = Onboarding::new(
        Progress {
            welcomed: true,
            tip_command: true,
            tip_spaces: true,
            tip_sidebar: true,
        },
        true,
    );
    h.key_press_modifiers(Modifiers::COMMAND, Key::L);
    h.run_steps(3);
    h.event(egui::Event::Text("welcome tour".into()));
    h.run_steps(3);
    h.key_press(Key::Enter);
    wait_label(&mut h, "Welcome to figstudio", true);
    assert_eq!(
        h.state().onboarding.progress,
        Progress::default(),
        "the tips come back too"
    );
}

#[test]
fn what_was_seen_is_kept_between_launches() {
    use eframe::App as _;
    #[derive(Default)]
    struct Memory(std::collections::HashMap<String, String>);
    impl eframe::Storage for Memory {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }
    let mut storage = Memory::default();
    let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    let mut first = App::new(&cc, Vec::new());
    let seen = Progress {
        welcomed: true,
        tip_command: true,
        ..Progress::default()
    };
    first.onboarding.progress = seen.clone();
    first.pages_share = 0.3;
    first.save(&mut storage);
    let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    cc.storage = Some(&storage);
    assert_eq!(App::new(&cc, Vec::new()).onboarding.progress, seen);

    // A state saved before onboarding existed still loads, with nothing seen yet.
    let saved = storage.0.get(crate::UI_STATE_KEY).cloned().expect("saved");
    let at = saved.find(",onboarding:").expect("the field");
    let end = at + saved[at..].find(')').expect("its end") + 1;
    let old = format!("{}{}", &saved[..at], &saved[end..]);
    storage.0.insert(crate::UI_STATE_KEY.to_owned(), old);
    let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    cc.storage = Some(&storage);
    let app = App::new(&cc, Vec::new());
    assert_eq!(app.onboarding.progress, Progress::default());
    assert!((app.pages_share - 0.3).abs() < 1e-6, "the rest loaded");
}

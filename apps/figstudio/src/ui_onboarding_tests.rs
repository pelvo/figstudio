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

/// How opaque the last frame painted the text `text` (its strongest glyph), if it did. egui
/// fades text by scaling its glyphs' colours, so that is what is read.
fn text_opacity(h: &Harness<'_, App>, text: &str) -> Option<f32> {
    fn walk(shape: &egui::Shape, text: &str) -> Option<f32> {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| walk(s, text)),
            egui::Shape::Text(t) if t.galley.job.text == text => {
                let alpha = (t.galley.rows.iter())
                    .flat_map(|r| r.row.visuals.mesh.vertices.iter())
                    .map(|v| v.color.a())
                    .max()?;
                Some(f32::from(alpha) / 255.0 * t.opacity_factor)
            }
            _ => None,
        }
    }
    h.output().shapes.iter().find_map(|c| walk(&c.shape, text))
}

#[test]
fn the_page_picture_fades_out_with_the_sheet() {
    // Frames 1/60 s apart, so the fade spans several of them.
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| App::new(cc, Vec::new()));
    h.state_mut().onboarding = Onboarding::new(Progress::default(), true);
    crate::anim::set_reduced(&h.ctx.clone(), false);
    wait_label(&mut h, "Welcome to figstudio", true);
    h.key_press(egui::Key::ArrowRight);
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(40);
    let picture = "Search or paste a Figma link";
    assert_eq!(text_opacity(&h, picture), Some(1.0), "the picture is up");
    h.key_press(egui::Key::Enter);
    // Every frame of the fade: the picture is never brighter than the sheet's own title.
    let mut faded = false;
    for _ in 0..60 {
        h.step();
        let (Some(pic), Some(title)) =
            (text_opacity(&h, picture), text_opacity(&h, "Find anything"))
        else {
            continue;
        };
        assert!(
            pic <= title + 0.001,
            "picture {pic} over a sheet at {title}"
        );
        faded |= pic < 0.9;
    }
    assert!(faded, "the picture faded with the sheet");
    assert!(text_opacity(&h, picture).is_none(), "and is gone with it");
}

/// The welcome sheet up again over `files` (Help → Welcome tour), all loaded.
fn sheet_over(files: &[&str]) -> Harness<'static, App> {
    let paths: Vec<PathBuf> = files.iter().map(|f| fixture(f)).collect();
    let n = paths.len();
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(move |cc| App::new(cc, paths));
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().tabs.len() < n {
        assert!(Instant::now() < deadline, "timed out loading {files:?}");
        h.step();
    }
    h.run_steps(4);
    h.state_mut().onboarding = Onboarding::new(Progress::default(), true);
    wait_label(&mut h, "Welcome to figstudio", true);
    h
}

fn wheel(delta: egui::Vec2, phase: egui::TouchPhase) -> egui::Event {
    egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta,
        phase,
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn the_sheet_keeps_the_wheel_and_pinch_off_the_canvas() {
    let mut h = sheet_over(&[FIXTURE]);
    let canvas = h.state().tabs[0].canvas_rect.expect("laid out");
    let before = h.state().tabs[0].view;
    // On the canvas, beside the sheet.
    h.event(egui::Event::PointerMoved(
        canvas.left_bottom() + egui::vec2(40.0, -40.0),
    ));
    h.event(wheel(egui::vec2(0.0, -120.0), egui::TouchPhase::Move));
    h.step();
    h.event(egui::Event::Zoom(1.5));
    h.run_steps(3);
    assert_eq!(h.state().tabs[0].view, before, "the page did not move");
}

#[test]
fn the_sheet_keeps_a_swipe_off_the_spaces() {
    let mut h = sheet_over(&[FIXTURE, "vector.fig"]);
    let active = h.state().active;
    h.event(egui::Event::PointerMoved(Pos2::new(120.0, 400.0)));
    h.event(wheel(egui::Vec2::ZERO, egui::TouchPhase::Start));
    for _ in 0..4 {
        h.event(wheel(egui::vec2(30.0, 0.0), egui::TouchPhase::Move));
        h.step();
    }
    h.event(wheel(egui::Vec2::ZERO, egui::TouchPhase::End));
    h.run_steps(3);
    assert_eq!(h.state().active, active, "no space switch under the sheet");
    assert!(!h.state().onboarding.progress.tip_spaces);
}

#[test]
fn the_sheet_keeps_typing_out_of_a_field_under_it() {
    let mut h = harness();
    h.key_press_modifiers(Modifiers::COMMAND, Key::F);
    h.run_steps(3);
    assert!(h.state().find.is_open());
    h.state_mut().onboarding = Onboarding::new(Progress::default(), true);
    wait_label(&mut h, "Welcome to figstudio", true);
    h.event(egui::Event::Text("abc".into()));
    h.key_press(Key::Space);
    h.run_steps(3);
    assert_eq!(
        h.state().find.query,
        "",
        "the find field under the sheet got nothing"
    );
}

#[test]
fn shortcuts_work_again_while_the_sheet_fades_out() {
    let mut h = sheet_over(&[FIXTURE]);
    crate::anim::set_reduced(&h.ctx.clone(), false);
    h.key_press(Key::Escape);
    h.key_press_modifiers(Modifiers::COMMAND, Key::L);
    h.run_steps(2);
    assert!(
        h.state().command.is_open(),
        "the command bar opens right away"
    );
}

#[test]
fn with_the_sidebar_hidden_the_sidebar_tip_comes_first() {
    let mut h = welcomed();
    h.state_mut().show_left = false;
    // The toggle itself is not a lesson here: it was hidden before the tips began.
    h.run_steps(2);
    h.state_mut().onboarding.progress.tip_sidebar = false;
    wait_label(&mut h, "Make room", true);
    assert!(h.query_by_label("Search everything").is_none());
    assert!(
        h.query_by_label("Tip 1 of 3").is_some(),
        "it is the first tip shown"
    );
    assert!(
        h.query_by_label_contains("to show the sidebar").is_some(),
        "the text says how to bring it back, not how to hide it"
    );
}

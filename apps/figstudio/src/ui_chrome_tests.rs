//! The Arc-style chrome through the real app: the sidebar and its title strip, the file
//! switcher, the tool pill and the command bar.
use egui::accesskit::Role;
use egui::{Key, Modifiers};

use super::*;

#[test]
fn hiding_the_sidebar_leaves_a_title_strip_whose_button_brings_it_back() {
    let mut h = harness();
    assert!(
        h.query_by_label("Page 1").is_some(),
        "the page list is in the sidebar"
    );
    h.get_by_label("Hide sidebar").click();
    h.run_steps(3);
    assert!(!h.state().show_left);
    assert!(
        h.query_by_label("Page 1").is_none(),
        "the page list went with it"
    );
    h.get_by_label("Show sidebar").click();
    h.run_steps(3);
    assert!(h.state().show_left);
    assert!(h.query_by_label("Page 1").is_some());
}

#[test]
fn each_open_file_gets_its_own_colour_and_its_dot_switches_to_it() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    h.run_steps(3);
    let tints: Vec<usize> = h.state().tabs.iter().map(|t| t.tint).collect();
    assert_ne!(tints[0], tints[1], "each file its own colour");
    let first = h.state().tabs[0].title();
    let other = (h.state().active + 1) % 2;
    let title = h.state().tabs[other].title();
    h.get_by_role_and_label(Role::Button, &title).click();
    h.run_steps(3);
    assert_eq!(h.state().active, other, "{first} / {title}");
}

#[test]
fn the_tool_pill_picks_tools_and_zooms() {
    let mut h = harness();
    h.get_by_label("Zoom tool").click();
    h.run_steps(3);
    assert_eq!(h.state().tool, crate::canvas::Tool::Zoom);
    let before = h.state().tabs[0].view.zoom;
    h.get_by_label("Zoom in").click();
    h.run_steps(3);
    let zoomed = h.state().tabs[0].view.zoom;
    assert!(
        (zoomed / before - 1.25).abs() < 1e-3,
        "{before} -> {zoomed}"
    );
    h.get_by_label("Zoom to fit").click();
    h.run_steps(3);
    assert!(
        (h.state().tabs[0].view.zoom - before).abs() < 1e-3,
        "fits again"
    );
}

#[test]
fn the_command_bar_opens_with_its_shortcut_and_runs_what_is_picked() {
    let mut h = harness();
    let fitted = h.state().tabs[0].view.zoom;
    h.state_mut().tabs[0].view.zoom = fitted * 4.0;
    h.key_press_modifiers(Modifiers::COMMAND, Key::L);
    h.run_steps(3);
    assert!(h.state().command.is_open());
    h.event(egui::Event::Text("zoom to fit".into()));
    h.run_steps(3);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(!h.state().command.is_open(), "closes after running");
    assert!(
        (h.state().tabs[0].view.zoom - fitted).abs() < 1e-3,
        "zoomed to fit"
    );

    h.get_by_label("Command bar").click();
    h.run_steps(3);
    assert!(
        h.state().command.is_open(),
        "the sidebar field opens it too"
    );
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(!h.state().command.is_open());
    assert_eq!(h.state().tabs[0].selection, None);
}

#[test]
fn reduce_motion_and_appearance_apply_to_the_whole_ui() {
    use crate::menu::{Action, Appearance};
    let mut h = harness();
    let ctx = h.ctx.clone();
    assert!(
        crate::anim::reduced(&ctx),
        "the harness starts without motion"
    );
    h.state_mut().apply(Action::ReduceMotion, &ctx);
    h.state_mut().apply(Action::ReduceMotion, &ctx);
    assert!(crate::anim::reduced(&ctx), "toggled twice: still off");
    h.state_mut().apply(Action::ReduceMotion, &ctx);
    assert!(!crate::anim::reduced(&ctx));
    h.state_mut()
        .apply(Action::Appearance(Appearance::Light), &ctx);
    h.run_steps(2);
    assert_eq!(ctx.theme(), egui::Theme::Light);
    h.state_mut()
        .apply(Action::Appearance(Appearance::Dark), &ctx);
    h.run_steps(2);
    assert_eq!(ctx.theme(), egui::Theme::Dark);
}

#[test]
fn a_space_dot_menu_changes_colour_icon_and_name_and_saves_them_for_the_file() {
    use crate::space_editor::Popover;
    use crate::spaces::{Style, Symbol};
    use crate::theme::Colour;
    let mut h = harness();
    let open = |h: &mut Harness<'_, App>, item: &str| {
        let name = h.state().tabs[0].space_name();
        h.get_by_role_and_label(Role::Button, &name)
            .click_secondary();
        h.run_steps(3);
        h.get_by_label(item).click();
        h.run_steps(3);
    };
    let kind = |h: &Harness<'_, App>| h.state().space_editing.as_ref().map(|e| e.kind);
    let saved = |h: &Harness<'_, App>| {
        let tab = &h.state().tabs[0];
        h.state()
            .spaces
            .lookup(tab.doc_key.as_deref(), &tab.title())
    };

    open(&mut h, "Edit theme colour…");
    assert_eq!(kind(&h), Some(Popover::Colour));
    h.get_by_label("Rose").click();
    h.run_steps(2);
    assert_eq!(
        h.state().tabs[0].colour(),
        Colour::Preset(4),
        "the window follows at once"
    );
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(kind(&h), None, "Escape closes it");

    open(&mut h, "Change space icon…");
    assert_eq!(kind(&h), Some(Popover::Icon));
    h.get_by_label("🚀").click();
    h.run_steps(2);
    assert_eq!(kind(&h), None, "picking an icon closes it");

    open(&mut h, "Rename space…");
    assert_eq!(kind(&h), Some(Popover::Rename));
    h.event(egui::Event::Text("Launch".into()));
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(kind(&h), None, "Enter keeps the name");
    assert_eq!(
        saved(&h),
        Some(Style {
            colour: Some(Colour::Preset(4)),
            symbol: Some(Symbol::Emoji("🚀".into())),
            name: Some("Launch".into()),
        })
    );
    assert!(
        h.query_by_role_and_label(Role::Button, "Launch").is_some(),
        "the dot is renamed"
    );

    open(&mut h, "Rename space…");
    h.event(egui::Event::Text(" and more".into()));
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().tabs[0].space_name(), "Launch", "Escape goes back");

    open(&mut h, "Reset space look");
    assert_eq!(saved(&h), None, "reset forgets the file");
    assert_eq!(h.state().tabs[0].style, Style::default());
}

#[test]
fn command_f_finds_text_on_the_page_and_steps_through_it() {
    let mut h = harness();
    h.key_press_modifiers(Modifiers::COMMAND, Key::F);
    h.run_steps(3);
    assert!(h.state().find.is_open());
    h.event(egui::Event::Text("abc".into()));
    h.run_steps(3);
    assert_eq!(
        h.state().tabs[0].find_hits.len(),
        2,
        "the layer and the instance"
    );
    assert_eq!(h.state().tabs[0].find_current, Some(0));
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(
        h.state().tabs[0].find_current,
        Some(1),
        "Enter: the next match"
    );
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::G);
    h.run_steps(3);
    assert_eq!(
        h.state().tabs[0].find_current,
        Some(0),
        "⇧⌘G: the previous one"
    );
    assert!(h.query_by_label_contains("1 of 2").is_some());
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(!h.state().find.is_open(), "Escape closes the bar");
    assert!(h.state().tabs[0].find_hits.is_empty(), "and the highlights");
}

#[test]
fn a_file_opens_with_the_style_saved_for_it() {
    use crate::spaces::{Style, Symbol};
    use crate::theme::Colour;
    let mut h = harness();
    let saved = Style {
        colour: Some(Colour::Custom([10, 120, 250])),
        symbol: Some(Symbol::Icon("component".into())),
        name: Some("Icons".into()),
    };
    h.state_mut().spaces.remember(None, "vector.fig", &saved);
    h.state_mut().open(fixture("vector.fig"));
    wait(&mut h, &mut Vec::new(), "the second file", |a| {
        a.tabs.len() == 2
    });
    assert_eq!(h.state().tabs[1].style, saved);
    assert_eq!(
        h.state().tabs[0].style,
        Style::default(),
        "others keep theirs"
    );
}

#[test]
fn the_sidebar_and_panels_come_back_as_they_were_left() {
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
    assert!(
        first.show_left && first.show_right,
        "shown on a first launch"
    );
    first.show_left = false;
    first.pages_share = 0.3;
    first.save(&mut storage);
    let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    cc.storage = Some(&storage);
    let second = App::new(&cc, Vec::new());
    assert!(!second.show_left, "the hidden sidebar stays hidden");
    assert!(second.show_right);
    assert!((second.pages_share - 0.3).abs() < 1e-6);
}

#[test]
fn a_sideways_swipe_over_the_sidebar_moves_between_spaces() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    h.run_steps(2);
    let swipe = |h: &mut Harness<'_, App>, dx: f32| {
        let wheel = |delta, phase| Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta,
            phase,
            modifiers: egui::Modifiers::NONE,
        };
        h.event(Event::PointerMoved(Pos2::new(120.0, 400.0)));
        h.event(wheel(egui::Vec2::ZERO, egui::TouchPhase::Start));
        for _ in 0..4 {
            h.event(wheel(egui::vec2(dx / 4.0, 0.0), egui::TouchPhase::Move));
            h.step();
        }
        h.event(wheel(egui::Vec2::ZERO, egui::TouchPhase::End));
        h.run_steps(2);
    };
    assert_eq!(h.state().active, 1, "the last file opened is active");
    swipe(&mut h, -120.0);
    assert_eq!(h.state().active, 1, "no space after the last");
    swipe(&mut h, 120.0);
    assert_eq!(
        h.state().active,
        0,
        "swiping right shows the previous space"
    );
    swipe(&mut h, -30.0);
    assert_eq!(h.state().active, 0, "a short swipe springs back");
    swipe(&mut h, -120.0);
    assert_eq!(h.state().active, 1);
}

#[test]
fn the_sidebar_keeps_its_width_while_spaces_slide() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    // With motion on, so the pages really move.
    crate::anim::set_reduced(&h.ctx.clone(), false);
    h.run_steps(30);
    let width = |h: &Harness<'_, App>| {
        egui::containers::panel::PanelState::load(&h.ctx, egui::Id::new(crate::chrome::SIDEBAR))
            .map(|s| s.size().x)
            .expect("the sidebar was shown")
    };
    let before = width(&h);
    let edge = Pos2::new(before, 400.0);
    h.event(Event::PointerMoved(edge));
    h.step();
    press(&h, edge, PointerButton::Primary, true);
    for i in 1..=4 {
        h.event(Event::PointerMoved(edge - egui::vec2(5.0 * i as f32, 0.0)));
        h.step();
    }
    press(
        &h,
        edge - egui::vec2(20.0, 0.0),
        PointerButton::Primary,
        false,
    );
    h.run_steps(5);
    let narrowed = width(&h);
    assert!(
        (narrowed - (before - 20.0)).abs() < 1.5,
        "{before} -> {narrowed}"
    );
    for _ in 0..3 {
        let other = (h.state().active + 1) % 2;
        let name = h.state().tabs[other].space_name();
        h.get_by_role_and_label(Role::Button, &name).click();
        for _ in 0..40 {
            h.step();
            let now = width(&h);
            assert!(
                (now - narrowed).abs() < 1.5,
                "the width moved to {now} from {narrowed}"
            );
        }
    }
}

#[test]
fn mid_swipe_both_spaces_show_and_the_colour_is_between_theirs() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    h.state_mut().active = 0;
    h.run_steps(3);
    let wheel = |delta, phase| Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta,
        phase,
        modifiers: egui::Modifiers::NONE,
    };
    h.event(Event::PointerMoved(Pos2::new(120.0, 400.0)));
    h.event(wheel(egui::Vec2::ZERO, egui::TouchPhase::Start));
    for _ in 0..4 {
        h.event(wheel(egui::vec2(-25.0, 0.0), egui::TouchPhase::Move));
        h.step();
    }
    h.step();
    let app = h.state();
    assert!(
        (app.slide + 100.0).abs() < 1.0,
        "the pages follow the fingers: {}",
        app.slide
    );
    assert!(
        h.query_all_by_label("Page 1").count() >= 2,
        "both spaces' page lists are on screen"
    );
    let dark = h.ctx.theme() == egui::Theme::Dark;
    let shown = app.tint_now(&h.ctx).top;
    let own = crate::theme::space_tint(app.tabs[0].colour(), dark).top;
    let next = crate::theme::space_tint(app.tabs[1].colour(), dark).top;
    assert!(
        shown != own && shown != next,
        "{shown:?} between {own:?} and {next:?}"
    );
    h.event(wheel(egui::Vec2::ZERO, egui::TouchPhase::End));
    h.run_steps(3);
    assert_eq!(h.state().active, 1, "past a third: the next space");
}

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
    h.key_press_modifiers(Modifiers::COMMAND, Key::K);
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
fn the_space_editor_changes_colour_icon_and_name_and_saves_them_for_the_file() {
    use crate::spaces::{Style, Symbol};
    use crate::theme::Colour;
    let mut h = harness();
    h.get_by_label("Edit space").click();
    h.run_steps(3);
    assert!(
        h.state().space_editing.is_some(),
        "the header opens the editor"
    );
    h.get_by_label("Rose").click();
    h.run_steps(2);
    h.get_by_label("🚀").click();
    h.run_steps(2);
    let field = h.get_by(|n| {
        n.role() == egui::accesskit::Role::TextInput && n.placeholder() == Some("structure.fig")
    });
    field.click();
    h.run_steps(2);
    h.event(egui::Event::Text("Launch".into()));
    h.run_steps(2);
    let tab = &h.state().tabs[0];
    assert_eq!(
        tab.colour(),
        Colour::Preset(4),
        "the window follows at once"
    );
    assert_eq!(tab.space_name(), "Launch");
    h.get_by_label("Done").click();
    h.run_steps(3);
    assert!(h.state().space_editing.is_none());
    let want = Style {
        colour: Some(Colour::Preset(4)),
        symbol: Some(Symbol::Emoji("🚀".into())),
        name: Some("Launch".into()),
    };
    let tab = &h.state().tabs[0];
    let saved = h
        .state()
        .spaces
        .lookup(tab.doc_key.as_deref(), &tab.title());
    assert_eq!(saved, Some(want));
    assert!(
        h.query_by_role_and_label(Role::Button, "Launch").is_some(),
        "the file dot is named after the space"
    );
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

//! Screen readers: every control the app shows has a name in the AccessKit tree, in each main
//! state (Recents, an open file, the command bar, find, every Settings tab and the menu).
//!
//! A control is a node that takes a click or focus, or has a control role. Painted widgets
//! name themselves with `Response::widget_info`; a control without a name is read as
//! "button" and nothing else. egui's scroll bars are left out: they have no name in egui, and
//! screen readers scroll the area itself.

use egui::accesskit::{Action, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::{SIZE, centre_of, harness, right_click};
use crate::App;
use crate::menu::Action as MenuAction;

/// Controls in `h`'s tree without a name: role, value and place, for the failure message.
fn unnamed(h: &Harness<'_, App>) -> Vec<String> {
    h.root()
        .children_recursive()
        .filter_map(|n| {
            let a = n.accesskit_node();
            let control = a.role() != Role::ScrollBar
                && (a.data().supports_action(Action::Click)
                    || a.data().supports_action(Action::Focus)
                    || matches!(
                        a.role(),
                        Role::Button
                            | Role::CheckBox
                            | Role::Switch
                            | Role::RadioButton
                            | Role::Slider
                            | Role::SpinButton
                            | Role::TextInput
                            | Role::MultilineTextInput
                            | Role::SearchInput
                            | Role::ComboBox
                            | Role::Link
                            | Role::Tab
                            | Role::MenuItem
                            | Role::TreeItem
                            | Role::ListBoxOption
                    ));
            let text = if a.label_comes_from_value() {
                a.value()
            } else {
                a.label()
            };
            let named = text.is_some_and(|t| !t.trim().is_empty());
            (control && !named && !a.is_hidden()).then(|| {
                format!(
                    "{:?} value={:?} at {:?}",
                    a.role(),
                    a.value(),
                    a.bounding_box()
                )
            })
        })
        .collect()
}

#[track_caller]
fn assert_all_named(h: &Harness<'_, App>, state: &str) {
    // Nodes the app names after egui made them (`a11y.rs`) must be ones egui did make: a
    // node added for a widget that is not there has no place on screen.
    let placeless: Vec<String> = h
        .root()
        .children_recursive()
        .map(|n| n.accesskit_node())
        .filter(|a| matches!(a.role(), Role::Splitter | Role::Menu | Role::Canvas))
        .filter(|a| a.bounding_box().is_none())
        .map(|a| format!("{:?} {:?}", a.role(), a.label()))
        .collect();
    assert!(
        placeless.is_empty(),
        "{state}: nodes without a place: {placeless:?}"
    );
    let missing = unnamed(h);
    assert!(
        missing.is_empty(),
        "{state}: {} control(s) without a name:\n{}",
        missing.len(),
        missing.join("\n")
    );
}

#[test]
fn every_control_on_the_recents_page_has_a_name() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, vec![]));
    h.run_steps(4);
    assert_all_named(&h, "recents");
}

#[test]
fn every_control_with_a_file_open_has_a_name() {
    let mut h = harness();
    h.run_steps(2);
    assert_all_named(&h, "file open");

    let ctx = h.ctx.clone();
    for (action, state) in [
        (MenuAction::CommandBar, "command bar"),
        (MenuAction::FindOnPage, "find bar"),
        (MenuAction::OpenLink, "open Figma link"),
        (MenuAction::RenameSpace, "space name"),
        (MenuAction::SpaceIcon, "space icon"),
        (MenuAction::SpaceColour, "space colour"),
        (MenuAction::Welcome, "welcome sheet"),
    ] {
        h.state_mut().apply(action, &ctx);
        h.run_steps(4);
        assert_all_named(&h, state);
        h.key_press(egui::Key::Escape);
        h.run_steps(30);
    }
}

#[test]
fn every_item_of_the_drawn_menu_has_a_name() {
    let mut h = harness();
    h.get_by_label("Menu").click();
    h.run_steps(4);
    assert_all_named(&h, "menu");
    // Its submenus too: View has the most items.
    h.get_by_label_contains("View").hover();
    h.run_steps(4);
    assert!(h.query_by_label("Reduce motion").is_some(), "View is open");
    assert_all_named(&h, "View menu");
}

#[test]
fn every_control_in_each_settings_tab_has_a_name() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().apply(MenuAction::Settings, &ctx);
    h.run_steps(4);
    for tab in crate::settings::Tab::ALL {
        h.state_mut().settings.tab = tab;
        h.run_steps(4);
        assert_all_named(&h, &format!("settings, {}", tab.label()));
    }
}

#[test]
fn a_right_click_menu_is_a_named_menu_of_named_items() {
    let mut h = harness();
    // A layer of structure.fig (the one the Copy menu tests use).
    let at = centre_of(&h, figstudio_fig::Guid(1, 34));
    right_click(&mut h, &mut Vec::new(), at);
    h.run_steps(4);
    let menus: Vec<_> = h
        .root()
        .children_recursive()
        .map(|n| n.accesskit_node())
        .filter(|a| a.role() == Role::Menu)
        .map(|a| a.label())
        .collect();
    assert_eq!(menus, [Some("Context menu".to_owned())]);
    assert_all_named(&h, "context menu");
}

#[test]
fn the_canvas_says_its_page_zoom_and_selection() {
    let mut h = harness();
    h.run_steps(2);
    let describe = |h: &Harness<'_, App>| {
        let canvas = h.get_by_role(Role::Canvas);
        let a = canvas.accesskit_node();
        (
            a.label().unwrap_or_default(),
            a.description().unwrap_or_default(),
        )
    };
    let tab = &h.state().tabs[0];
    let page = crate::panels::display_name(tab.doc.node(tab.page).expect("page"));
    let zoom = (tab.view.zoom * 100.0).round();
    let (label, description) = describe(&h);
    assert_eq!(label, format!("Canvas, page {page}"));
    assert_eq!(description, format!("Zoom {zoom}%. Selected: nothing."));

    let node = figstudio_fig::Guid(1, 34);
    h.state_mut().tabs[0].select(Some(node));
    h.run_steps(2);
    let name = crate::panels::display_name(h.state().tabs[0].doc.node(node).expect("node"));
    let (_, description) = describe(&h);
    assert!(
        description.ends_with(&format!("Selected: {name}.")),
        "{description}"
    );
}

#[test]
fn a_layer_with_children_says_whether_it_is_open_and_its_arrow_toggles_it() {
    let mut h = harness();
    h.run_steps(2);
    let state = |h: &Harness<'_, App>, verb: &str| {
        h.root()
            .children_recursive()
            .map(|n| n.accesskit_node())
            .find_map(|a| {
                let label = a.label()?;
                let name = label.strip_prefix(verb)?.trim_start().to_owned();
                Some(name)
            })
    };
    let name = state(&h, "Expand").expect("a closed layer with children");
    let row = |h: &Harness<'_, App>| {
        h.root()
            .children_recursive()
            .map(|n| n.accesskit_node())
            .find(|a| a.label().as_deref() == Some(name.as_str()))
            .and_then(|a| a.data().is_expanded())
    };
    assert_eq!(row(&h), Some(false));
    h.get_by_label(&format!("Expand {name}")).click();
    h.run_steps(4);
    assert_eq!(row(&h), Some(true));
    assert!(h.query_by_label(&format!("Collapse {name}")).is_some());
}

/// True when `shapes` hold an accent outline (the focus ring) around `rect`.
fn ring_around(shapes: &[egui::epaint::ClippedShape], rect: egui::Rect) -> bool {
    fn walk(shape: &egui::Shape, rect: egui::Rect) -> bool {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().any(|s| walk(s, rect)),
            egui::Shape::Rect(r) => {
                r.stroke.color == crate::theme::ACCENT
                    && r.stroke.width >= 1.5
                    && r.rect.expand(4.0).contains_rect(rect)
                    && rect.expand(4.0).contains_rect(r.rect)
            }
            _ => false,
        }
    }
    shapes.iter().any(|c| walk(&c.shape, rect))
}

/// Tabs through every control of `h` once and returns the ones that show no focus ring
/// (text fields show their caret instead).
fn focus_without_ring(h: &mut Harness<'_, App>) -> Vec<String> {
    let ctx = h.ctx.clone();
    let mut seen = std::collections::HashSet::new();
    let mut missing = Vec::new();
    for _ in 0..200 {
        h.key_press(egui::Key::Tab);
        h.run_steps(2);
        let Some(id) = ctx.memory(|m| m.focused()) else {
            continue;
        };
        if !seen.insert(id) {
            break;
        }
        if egui::TextEdit::load_state(&ctx, id).is_some() {
            continue;
        }
        let Some(response) = ctx.read_response(id) else {
            continue;
        };
        if !ring_around(&h.output().shapes, response.rect) {
            let name = h
                .root()
                .children_recursive()
                .map(|n| n.accesskit_node())
                .find(|a| a.is_focused())
                .and_then(|a| a.label().or_else(|| a.value()))
                .unwrap_or_default();
            missing.push(format!("{name:?} at {:?}", response.rect));
        }
    }
    assert!(seen.len() > 5, "Tab reached {} controls", seen.len());
    missing
}

#[test]
fn every_control_tab_reaches_shows_a_focus_ring() {
    let mut h = harness();
    h.run_steps(2);
    let missing = focus_without_ring(&mut h);
    assert!(missing.is_empty(), "no focus ring:\n{}", missing.join("\n"));

    let ctx = h.ctx.clone();
    h.state_mut().apply(MenuAction::Settings, &ctx);
    h.run_steps(4);
    let missing = focus_without_ring(&mut h);
    assert!(
        missing.is_empty(),
        "settings, no focus ring:\n{}",
        missing.join("\n")
    );
}

#[test]
fn every_control_on_the_recents_page_shows_a_focus_ring() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, vec![]));
    h.run_steps(4);
    let missing = focus_without_ring(&mut h);
    assert!(missing.is_empty(), "no focus ring:\n{}", missing.join("\n"));
}

/// The layers Tab reaches in `h`, one walk round the controls.
fn tab_layers(h: &mut Harness<'_, App>) -> Vec<(egui::Order, String)> {
    let ctx = h.ctx.clone();
    let mut seen = std::collections::HashSet::new();
    let mut layers = Vec::new();
    for _ in 0..200 {
        h.key_press(egui::Key::Tab);
        h.run_steps(2);
        let Some(id) = ctx.memory(|m| m.focused()) else {
            continue;
        };
        if !seen.insert(id) {
            break;
        }
        let name = h
            .root()
            .children_recursive()
            .map(|n| n.accesskit_node())
            .find(|a| a.is_focused())
            .and_then(|a| a.label().or_else(|| a.value()))
            .unwrap_or_default();
        if let Some(r) = ctx.read_response(id) {
            layers.push((r.layer_id.order, name));
        }
    }
    layers
}

#[test]
fn tab_stays_inside_a_modal_dialog() {
    // The welcome sheet takes its keys itself (Enter, arrows, Escape) and holds no focus.
    for (action, state, reaches) in [
        (MenuAction::Settings, "settings", true),
        (MenuAction::CommandBar, "command bar", true),
        (MenuAction::Welcome, "welcome sheet", false),
    ] {
        let mut h = harness();
        let ctx = h.ctx.clone();
        h.state_mut().apply(action, &ctx);
        h.run_steps(6);
        let layers = tab_layers(&mut h);
        assert_eq!(
            !layers.is_empty(),
            reaches,
            "{state}: Tab reaches {layers:?}"
        );
        let behind: Vec<_> = layers
            .iter()
            .filter(|(order, _)| *order != egui::Order::Foreground)
            .map(|(_, name)| name.as_str())
            .collect();
        assert!(
            behind.is_empty(),
            "{state}: Tab leaves the dialog for {behind:?}"
        );
    }
}

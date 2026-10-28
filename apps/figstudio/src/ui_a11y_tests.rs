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

/// A node a screen reader or agent can act on: it takes a click or focus, or has a control
/// role. egui's scroll bars are left out.
fn is_control(a: &egui_kittest::kittest::AccessKitNode<'_>) -> bool {
    a.role() != Role::ScrollBar
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
            ))
}

/// Controls in `h`'s tree without a name: role, value and place, for the failure message.
fn unnamed(h: &Harness<'_, App>) -> Vec<String> {
    h.root()
        .children_recursive()
        .filter_map(|n| {
            let a = n.accesskit_node();
            let control = is_control(&a);
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

/// Controls (text aside) without a stable automation id (`author_id`, `AXIdentifier` on macOS),
/// and ids used twice.
fn unidentified(h: &Harness<'_, App>) -> (Vec<String>, Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    let (mut missing, mut twice) = (Vec::new(), Vec::new());
    for n in h.root().children_recursive() {
        let a = n.accesskit_node();
        if a.is_hidden() {
            continue;
        }
        match a.data().author_id() {
            Some(id) => {
                if !seen.insert(id.to_owned()) {
                    twice.push(id.to_owned());
                }
            }
            // egui's own window close button has an id we cannot know (`auto_id_with`).
            None if a.label().as_deref() == Some("Close window") => {}
            None if is_control(&a) && a.role() != Role::Label => missing.push(format!(
                "{:?} {:?} at {:?}",
                a.role(),
                a.label().or_else(|| a.value()),
                a.bounding_box().map(|b| (b.x0 as i32, b.y0 as i32))
            )),
            None => {}
        }
    }
    (missing, twice)
}

#[track_caller]
fn assert_all_named(h: &Harness<'_, App>, state: &str) {
    let (missing, twice) = unidentified(h);
    assert!(
        missing.is_empty(),
        "{state}: {} control(s) without an automation id:\n{}",
        missing.len(),
        missing.join("\n")
    );
    assert!(
        twice.is_empty(),
        "{state}: automation ids used twice: {twice:?}"
    );
    // Nodes the app names after egui made them (`a11y.rs`) must be ones egui did make: a
    // node added for a widget that is not there has no place on screen.
    let placeless: Vec<String> = h
        .root()
        .children_recursive()
        .map(|n| n.accesskit_node())
        .filter(|a| {
            matches!(a.role(), Role::Splitter | Role::Menu) || a.data().author_id().is_some()
        })
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
        let a = h
            .root()
            .children_recursive()
            .map(|n| n.accesskit_node())
            .find(|a| a.data().author_id() == Some("canvas"))
            .expect("the canvas node");
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

/// True when `shapes` hold a focus-coloured outline (the focus ring) around `rect`.
fn ring_around(shapes: &[egui::epaint::ClippedShape], rect: egui::Rect) -> bool {
    fn walk(shape: &egui::Shape, rect: egui::Rect) -> bool {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().any(|s| walk(s, rect)),
            egui::Shape::Rect(r) => {
                [crate::theme::LIGHT.focus, crate::theme::DARK.focus].contains(&r.stroke.color)
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

#[test]
fn command_plus_and_minus_scale_the_whole_ui_and_leave_the_canvas_alone() {
    use egui::{Key, Modifiers};
    let mut h = harness();
    h.run_steps(2);
    let ctx = h.ctx.clone();
    let canvas = h.state().tabs[0].view.zoom;
    for key in [Key::Plus, Key::Equals] {
        let before = ctx.zoom_factor();
        h.key_press_modifiers(Modifiers::COMMAND, key);
        h.run_steps(2);
        assert!(ctx.zoom_factor() > before, "{key:?}: {}", ctx.zoom_factor());
        assert_eq!(
            h.state().tabs[0].view.zoom,
            canvas,
            "{key:?} zooms the canvas too"
        );
    }
    h.key_press_modifiers(Modifiers::COMMAND, Key::Minus);
    h.run_steps(2);
    assert!(
        (ctx.zoom_factor() - 1.1).abs() < 1e-3,
        "{}",
        ctx.zoom_factor()
    );
    h.key_press_modifiers(Modifiers::COMMAND, Key::Num0);
    h.run_steps(2);
    assert_eq!(ctx.zoom_factor(), 1.0);
    assert_eq!(h.state().tabs[0].view.zoom, canvas);
}

#[test]
fn settings_sets_the_text_size() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    let canvas = h.state().tabs[0].view.zoom;
    h.state_mut().apply(MenuAction::Settings, &ctx);
    h.run_steps(4);
    h.get_by_role_and_label(Role::RadioButton, "150%").click();
    h.run_steps(4);
    assert_eq!(ctx.zoom_factor(), 1.5);
    assert_eq!(h.state().tabs[0].view.zoom, canvas);
    h.get_by_role_and_label(Role::RadioButton, "100%").click();
    h.run_steps(4);
    assert_eq!(ctx.zoom_factor(), 1.0);
}

#[test]
fn action_buttons_are_plain_buttons_and_tools_are_toggles() {
    let mut h = harness();
    h.run_steps(2);
    let toggled =
        |h: &Harness<'_, App>, label: &str| h.get_by_label(label).accesskit_node().data().toggled();
    for action in ["Zoom in", "Zoom out", "Open file…", "Hide sidebar"] {
        assert_eq!(toggled(&h, action), None, "{action} is read as a toggle");
    }
    for tool in ["Select tool", "Hand tool", "Zoom tool"] {
        assert!(
            toggled(&h, tool).is_some(),
            "{tool} does not say whether it is on"
        );
    }
}

#[test]
fn closing_a_modal_gives_focus_back_to_what_had_it() {
    for (action, state) in [
        (MenuAction::Settings, "settings"),
        (MenuAction::CommandBar, "command bar"),
        (MenuAction::Welcome, "welcome sheet"),
    ] {
        let mut h = harness();
        h.run_steps(2);
        let ctx = h.ctx.clone();
        // Focus the Zoom in button the way Tab does.
        let zoom_in = ctx
            .read_response(egui::Id::new("zoom-in"))
            .expect("the Zoom in button")
            .id;
        ctx.memory_mut(|m| m.request_focus(zoom_in));
        h.run_steps(2);
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(zoom_in),
            "{state}: focused"
        );
        h.state_mut().apply(action, &ctx);
        h.run_steps(6);
        h.key_press(egui::Key::Escape);
        h.run_steps(30);
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(zoom_in),
            "{state}: focus is not back on Zoom in"
        );
    }
}

#[test]
fn a_choice_of_one_reads_as_radio_buttons_and_settings_sections_as_tabs() {
    let mut h = harness();
    h.run_steps(2);
    for tool in ["Select tool", "Hand tool", "Zoom tool"] {
        assert_eq!(
            h.get_by_label(tool).accesskit_node().role(),
            Role::RadioButton,
            "{tool}"
        );
    }
    let ctx = h.ctx.clone();
    h.state_mut().apply(MenuAction::Settings, &ctx);
    h.run_steps(4);
    for choice in ["System", "Light", "Dark", "100%", "150%"] {
        assert_eq!(
            h.get_by_label(choice).accesskit_node().role(),
            Role::RadioButton,
            "{choice}"
        );
    }
    let general = h.get_by_role_and_label(Role::Tab, "General");
    assert_eq!(general.accesskit_node().is_selected(), Some(true));
    assert_eq!(general.accesskit_node().data().toggled(), None);
    assert_eq!(
        h.get_by_role_and_label(Role::Tab, "Canvas")
            .accesskit_node()
            .is_selected(),
        Some(false)
    );
}

#[test]
fn the_text_size_survives_a_restart() {
    use eframe::App as _;
    for (button, size) in [("125%", 1.25), ("150%", 1.5), ("200%", 2.0)] {
        let mut h = harness();
        let ctx = h.ctx.clone();
        h.state_mut().apply(MenuAction::Settings, &ctx);
        h.run_steps(4);
        h.get_by_role_and_label(Role::RadioButton, button).click();
        h.run_steps(4);
        let mut storage = super::Memory::default();
        h.state_mut().save(&mut storage);

        let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        cc.storage = Some(&storage);
        let second = App::new(&cc, Vec::new());
        assert_eq!(second.prefs.text_size, size, "{button}");
        // egui applies a new zoom at the start of its next pass.
        let _ = cc.egui_ctx.run_ui(egui::RawInput::default(), |_| {});
        assert_eq!(cc.egui_ctx.zoom_factor(), size, "{button}");
    }
    // ⌘+ is kept too, and Settings says what it is.
    let mut h = harness();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Plus);
    h.run_steps(3);
    let mut storage = super::Memory::default();
    h.state_mut().save(&mut storage);
    let cc = {
        let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        cc.storage = Some(&storage);
        let _second = App::new(&cc, Vec::new());
        let _ = cc.egui_ctx.run_ui(egui::RawInput::default(), |_| {});
        cc.egui_ctx.zoom_factor()
    };
    assert!((cc - 1.1).abs() < 1e-3, "{cc}");
    let ctx = h.ctx.clone();
    h.state_mut().apply(MenuAction::Settings, &ctx);
    h.run_steps(4);
    assert!(h.query_by_label_contains("Now 110%").is_some());
}

/// The layer nodes under the canvas.
fn canvas_layers<'h>(h: &'h Harness<'_, App>) -> Vec<egui_kittest::kittest::AccessKitNode<'h>> {
    h.root()
        .children_recursive()
        .map(|n| n.accesskit_node())
        .filter(|a| {
            a.data()
                .author_id()
                .is_some_and(|id| id.starts_with("canvas.layer."))
        })
        .collect()
}

#[test]
fn the_canvas_lists_its_visible_layers_by_name_kind_and_place() {
    let mut h = harness();
    h.run_steps(2);
    let canvas = h.state().tabs[0].canvas_rect.expect("laid out");
    let layers = canvas_layers(&h);
    assert!(layers.len() > 5, "{} layers", layers.len());
    let frame = layers
        .iter()
        .find(|a| a.label().as_deref() == Some("Groups"))
        .expect("the Groups frame");
    assert_eq!(frame.role(), Role::Group);
    assert_eq!(frame.role_description(), Some("frame"));
    // Its layers are its children in the tree, not the canvas's.
    assert!(frame.children().count() > 0, "Groups has no children");
    let text = layers
        .iter()
        .find(|a| a.role() == Role::Label)
        .expect("a text layer");
    assert!(text.value().is_some_and(|v| !v.is_empty()));
    for a in &layers {
        let b = a.bounding_box().expect("a place on screen");
        let r = egui::Rect::from_min_max(
            egui::pos2(b.x0 as f32, b.y0 as f32),
            egui::pos2(b.x1 as f32, b.y1 as f32),
        );
        assert!(
            canvas.expand(0.5).contains_rect(r),
            "{:?} at {r:?}",
            a.label()
        );
        assert!(a.data().supports_action(egui::accesskit::Action::Click));
    }
}

#[test]
fn pressing_a_canvas_layer_selects_it() {
    let mut h = harness();
    h.run_steps(2);
    let node = figstudio_fig::Guid(1, 34);
    let id = crate::canvas_a11y::author_id(node);
    let (target_node, target_tree) = canvas_layers(&h)
        .into_iter()
        .find(|a| a.data().author_id() == Some(id.as_str()))
        .expect("the layer is listed")
        .locate();
    h.input_mut()
        .events
        .push(egui::Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                target_node,
                target_tree,
                action: egui::accesskit::Action::Click,
                data: None,
            },
        ));
    h.run_steps(3);
    assert_eq!(h.state().tabs[0].selection, Some(node));
    let selected: Vec<_> = canvas_layers(&h)
        .into_iter()
        .filter(|a| a.is_selected() == Some(true))
        .map(|a| a.data().author_id().map(str::to_owned))
        .collect();
    assert_eq!(selected, [Some(id)]);
}

#[test]
fn layers_off_the_screen_are_not_listed() {
    let mut h = harness();
    h.run_steps(2);
    let all = canvas_layers(&h).len();
    assert!(all > 0, "nothing listed to begin with");
    // Far off to the side: nothing of the page is on screen.
    h.state_mut().tabs[0].view.pan += egui::vec2(100_000.0, 0.0);
    h.run_steps(2);
    assert_eq!(canvas_layers(&h).len(), 0, "of {all}");
}

#[test]
fn a_properties_row_reads_as_key_and_value_together() {
    let mut h = harness();
    h.run_steps(2);
    let values: Vec<String> = h
        .root()
        .children_recursive()
        .map(|n| n.accesskit_node())
        .filter(|a| a.role() == Role::Label && !a.is_hidden())
        .filter_map(|a| a.value())
        .collect();
    let layers = h.state().tabs[0].doc.nodes.len();
    assert!(values.contains(&format!("Layers: {layers}")), "{values:?}");
    assert!(
        !values.iter().any(|v| v == "Layers"),
        "a lone key: {values:?}"
    );

    h.state_mut().tabs[0].select(Some(figstudio_fig::Guid(1, 34)));
    h.run_steps(3);
    let values: Vec<String> = h
        .root()
        .children_recursive()
        .map(|n| n.accesskit_node())
        .filter(|a| a.role() == Role::Label && !a.is_hidden())
        .filter_map(|a| a.value())
        .collect();
    for key in ["X", "Y", "W", "H"] {
        assert!(
            values.iter().any(|v| v.starts_with(&format!("{key}: "))),
            "no {key}: {values:?}"
        );
        assert!(!values.iter().any(|v| v == key), "a lone {key}: {values:?}");
    }
}

#[test]
fn toasts_are_announced_and_errors_interrupt() {
    use egui::accesskit::Live;
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().toast(&ctx, "Copied the link", false);
    h.state_mut().toast(&ctx, "Could not open the file", true);
    h.run_steps(3);
    let live = |h: &Harness<'_, App>, text: &str| {
        h.root()
            .children_recursive()
            .map(|n| n.accesskit_node())
            .find(|a| a.value().as_deref() == Some(text) || a.label().as_deref() == Some(text))
            .map(|a| (a.role(), a.live()))
    };
    assert_eq!(
        live(&h, "Copied the link"),
        Some((Role::Label, Live::Polite))
    );
    assert_eq!(
        live(&h, "Could not open the file"),
        Some((Role::Alert, Live::Assertive))
    );
}

#[test]
fn automation_ids_stay_the_same_when_labels_change() {
    let mut h = harness();
    h.run_steps(2);
    let label_of = |h: &Harness<'_, App>, id: &str| {
        h.root()
            .children_recursive()
            .map(|n| n.accesskit_node())
            .find(|a| a.data().author_id() == Some(id))
            .map(|a| a.label().unwrap_or_default())
    };
    for id in [
        "canvas",
        "button.sidebar",
        "button.command-bar",
        "button.select-tool",
        "button.zoom-in",
        "button.zoom-to-fit",
        "field.filter-layers",
        "splitter.properties",
    ] {
        assert!(label_of(&h, id).is_some(), "no {id}");
    }
    let tab = &h.state().tabs[0];
    let first = tab.doc.children(tab.page)[0];
    assert!(label_of(&h, &format!("layers.{first}")).is_some());
    assert!(label_of(&h, &format!("pages.{}", tab.page)).is_some());

    // The sidebar button reads "Hide sidebar" here and "Show sidebar" in the title strip.
    assert_eq!(
        label_of(&h, "button.sidebar").as_deref(),
        Some("Hide sidebar")
    );
    h.state_mut().show_left = false;
    h.run_steps(6);
    assert_eq!(
        label_of(&h, "button.sidebar").as_deref(),
        Some("Show sidebar")
    );
}

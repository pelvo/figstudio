//! Prototype Preview driven through the app: keys, clicks on links, Back and Esc.
#[path = "../../../crates/render/tests/common/prototype.rs"]
mod proto;

use std::sync::Arc;

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::{harness, step};
use crate::App;
use crate::ui_tests::pick::click;
use proto::{BACK, FIRST, LINK, SECOND};

fn previewing() -> Harness<'static, App> {
    let mut h = harness();
    h.state_mut().tabs[0].doc = Arc::new(proto::prototype_doc());
    h.run_steps(4);
    h
}

fn screen(h: &Harness<'_, App>) -> Option<figstudio_fig::Guid> {
    h.state().tabs[0]
        .preview
        .as_ref()
        .map(|p| p.player.screen())
}

#[test]
fn preview_follows_links_goes_back_and_esc_returns_to_the_canvas() {
    let mut h = previewing();
    let canvas_view = h.state().tabs[0].view;
    h.key_press_modifiers(Modifiers::SHIFT, Key::Space);
    h.run_steps(4);
    assert_eq!(screen(&h), Some(FIRST), "⇧Space starts at the flow");
    let tab = &h.state().tabs[0];
    assert_eq!(tab.root(), FIRST);
    assert!(
        !tab.tile_requests.is_empty() && tab.tile_requests.iter().all(|k| k.page == FIRST),
        "the tiles draw the screen alone"
    );
    let _ = h.get_by_label("Exit");

    let link = super::centre_of(&h, LINK);
    click(&mut h, link, Modifiers::NONE);
    assert_eq!(screen(&h), Some(SECOND), "a click on the link navigates");

    let corner = h.state().tabs[0].canvas_rect.unwrap().left_top() + egui::vec2(4.0, 4.0);
    click(&mut h, corner, Modifiers::NONE);
    assert_eq!(screen(&h), Some(SECOND), "a click on no link stays");
    assert!(
        h.state().tabs[0].preview.as_ref().unwrap().missed.is_some(),
        "and flashes the links"
    );

    let back = super::centre_of(&h, BACK);
    click(&mut h, back, Modifiers::NONE);
    assert_eq!(screen(&h), Some(FIRST), "the Back link goes back");

    click(&mut h, link, Modifiers::NONE);
    h.key_press(Key::Backspace);
    h.run_steps(2);
    assert_eq!(screen(&h), Some(FIRST), "Backspace goes back too");

    h.key_press(Key::Escape);
    h.run_steps(2);
    let tab = &h.state().tabs[0];
    assert_eq!(screen(&h), None, "Esc stops the preview");
    assert_eq!(tab.view, canvas_view, "the canvas view comes back");
    step(&mut h, &mut Vec::new());
}

#[test]
fn preview_starts_at_the_selected_screen_and_the_menu_toggles_it() {
    let mut h = previewing();
    h.state_mut().tabs[0].select(Some(BACK));
    let ctx = h.ctx.clone();
    h.state_mut().apply(crate::menu::Action::Preview, &ctx);
    h.run_steps(2);
    assert_eq!(screen(&h), Some(SECOND));
    assert_eq!(
        h.state().tabs[0].selection,
        None,
        "nothing is selected while previewing"
    );
    h.state_mut().apply(crate::menu::Action::Preview, &ctx);
    assert_eq!(screen(&h), None);
}

#[test]
fn esc_restores_the_canvas_after_preview_crossed_pages() {
    use figstudio_fig::{Guid, Node, Val};
    let mut h = previewing();
    let original_page = h.state().tabs[0].page;
    let original_view = h.state().tabs[0].view;
    let other_page = Guid(99, 100);
    let mut doc = proto::prototype_doc();
    let mut nodes = doc.nodes.clone();
    nodes.push(
        Node::from_data(proto::fields(&[
            ("guid", other_page.to_val()),
            ("type", proto::name("CANVAS")),
            ("name", Val::Str("Other page".into())),
            (
                "parentIndex",
                proto::fields(&[
                    ("guid", doc.root.unwrap().to_val()),
                    ("position", Val::Str("b".into())),
                ]),
            ),
        ]))
        .unwrap(),
    );
    let target = nodes.iter_mut().find(|n| n.id == SECOND).unwrap();
    target.data.set(
        "parentIndex",
        proto::fields(&[
            ("guid", other_page.to_val()),
            ("position", Val::Str("a".into())),
        ]),
    );
    *target = Node::from_data(target.data.clone()).unwrap();
    doc.set_nodes(nodes).unwrap();
    h.state_mut().tabs[0].doc = Arc::new(doc);
    h.key_press_modifiers(Modifiers::SHIFT, Key::Space);
    h.run_steps(4);
    assert_eq!(screen(&h), Some(FIRST));
    let link = super::centre_of(&h, LINK);
    click(&mut h, link, Modifiers::NONE);
    assert_eq!(screen(&h), Some(SECOND));
    assert_eq!(h.state().tabs[0].page, other_page);
    h.key_press(Key::Escape);
    h.run_steps(2);
    let tab = &h.state().tabs[0];
    assert_eq!(screen(&h), None);
    assert_eq!(
        (tab.page, tab.view),
        (original_page, original_view),
        "Esc restores the canvas we entered Preview from"
    );
}

#[test]
fn scaling_sizes_the_screen_as_figma_does() {
    use crate::preview::{Scaling, view_for};
    let screen = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(400.0, 1000.0));
    let window = egui::vec2(800.0, 500.0);
    let at = |s| {
        let v = view_for(screen, window, s);
        (v.zoom, v.to_screen(egui::Pos2::ZERO, screen.min))
    };
    // Figma's Fit keeps 48 points at the sides and 60 at the top and bottom.
    assert_eq!(
        at(Scaling::Fit),
        (0.38, egui::pos2(324.0, 60.0)),
        "whole screen inside the margins, centred"
    );
    assert_eq!(
        at(Scaling::FitWidth),
        (1.0, egui::pos2(200.0, 0.0)),
        "a screen narrower than the window is not enlarged"
    );
    let small = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 50.0));
    assert_eq!(
        view_for(small, window, Scaling::Fit).zoom,
        1.0,
        "Fit only scales down, as Figma's"
    );
    assert_eq!(at(Scaling::Fill), (2.0, egui::pos2(0.0, 0.0)));
    assert_eq!(
        at(Scaling::ActualSize),
        (1.0, egui::pos2(200.0, 0.0)),
        "1:1, centred across"
    );
}

#[test]
fn arrows_step_through_the_page_frames_and_present_hides_the_editor() {
    // Real frame times, so the panels slide away over several frames as they do on screen.
    let mut h = super::harness_fixture(super::FIXTURE, 1.0 / 60.0);
    h.state_mut().tabs[0].doc = Arc::new(proto::prototype_doc());
    h.run_steps(4);
    let ctx = h.ctx.clone();
    h.state_mut().apply(crate::menu::Action::Present, &ctx);
    h.run_steps(60);
    let tab = &h.state().tabs[0];
    assert!(tab.preview.as_ref().is_some_and(|p| p.present));
    let canvas = tab.canvas_rect.unwrap();
    assert!(
        canvas.width() >= super::SIZE.x - 1.0,
        "Present fills the window: {canvas:?}"
    );
    assert_eq!(screen(&h), Some(FIRST));
    let b = figstudio_render::node_bounds(&tab.doc, FIRST).unwrap();
    let first = egui::Rect::from_min_max(
        egui::pos2(b.x0 as f32, b.y0 as f32),
        egui::pos2(b.x1 as f32, b.y1 as f32),
    );
    assert_eq!(
        tab.view,
        crate::preview::view_for(first, canvas.size(), crate::preview::Scaling::Fit),
        "fitted to the canvas Present ends up with, after the panels are gone"
    );
    h.key_press(Key::ArrowRight);
    h.run_steps(2);
    assert_eq!(screen(&h), Some(SECOND), "→ is the next frame");
    h.key_press(Key::ArrowRight);
    h.run_steps(2);
    assert_eq!(screen(&h), Some(SECOND), "the last frame stays");
    h.key_press(Key::ArrowLeft);
    h.run_steps(2);
    assert_eq!(screen(&h), Some(FIRST), "← is the previous frame");
    h.key_press(Key::Escape);
    h.run_steps(4);
    assert_eq!(screen(&h), None);
    assert!(
        h.state().tabs[0].canvas_rect.unwrap().width() < super::SIZE.x - 100.0,
        "the editor is back"
    );
}

#[test]
fn arrows_work_after_a_text_field_had_the_keys() {
    let mut h = previewing();
    // As after the link dialog: a field it closed still holds the keyboard.
    h.ctx
        .memory_mut(|m| m.request_focus(egui::Id::new("closed-field")));
    h.key_press_modifiers(Modifiers::SHIFT, Key::Space);
    h.run_steps(4);
    assert_eq!(screen(&h), Some(FIRST));
    h.key_press(Key::ArrowRight);
    h.run_steps(2);
    assert_eq!(
        screen(&h),
        Some(SECOND),
        "→ steps even though a field had the keys"
    );
}

#[test]
fn hovering_a_button_shows_its_hover_variant_and_leaving_restores_it() {
    let mut h = harness();
    h.state_mut().tabs[0].doc = Arc::new(proto::hover_doc());
    h.run_steps(4);
    let ctx = h.ctx.clone();
    h.state_mut().apply(crate::menu::Action::Preview, &ctx);
    h.run_steps(4);
    let own = h.state().tabs[0].doc.instance;
    assert_eq!(h.state().tabs[0].tile_doc(), own);
    let button = super::centre_of(&h, proto::BUTTON);
    h.event(egui::Event::PointerMoved(button));
    h.run_steps(2);
    let tab = &h.state().tabs[0];
    assert_eq!(
        tab.prototype_state().swaps.get(&vec![proto::BUTTON]),
        Some(&proto::HOVERED),
        "hovered: the button shows its hover variant"
    );
    assert_ne!(tab.tile_doc(), own, "the tiles draw the hover state");
    let away = tab.canvas_rect.unwrap().left_top() + egui::vec2(4.0, 4.0);
    h.event(egui::Event::PointerMoved(away));
    h.run_steps(2);
    assert!(
        h.state().tabs[0].prototype_state().swaps.is_empty(),
        "left: back to the default"
    );
    assert_eq!(
        h.state().tabs[0].tile_doc(),
        own,
        "and the document's own tiles"
    );
}

#[test]
fn the_canvas_has_a_present_button_and_a_menu_with_preview() {
    let mut h = previewing();
    h.get_by_label("Present").click();
    h.run_steps(4);
    assert!(
        h.state().tabs[0]
            .preview
            .as_ref()
            .is_some_and(|p| p.present),
        "the play button starts Present"
    );
    h.key_press(Key::Escape);
    h.run_steps(4);
    h.get_by_label("Prototype options").click();
    h.run_steps(2);
    h.get_by_label_contains("Preview").click();
    h.run_steps(4);
    assert!(
        h.state().tabs[0]
            .preview
            .as_ref()
            .is_some_and(|p| !p.present),
        "the menu's Preview starts Preview"
    );
}

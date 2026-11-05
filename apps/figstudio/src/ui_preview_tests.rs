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
    h.key_press(Key::ArrowLeft);
    h.run_steps(2);
    assert_eq!(screen(&h), Some(FIRST), "← goes back too");

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

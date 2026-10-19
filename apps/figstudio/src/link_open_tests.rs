//! Open Figma link: which tab or local copy a link picks, and the app showing its node.
use std::path::{Path, PathBuf};

use figstudio_fig::Guid;

use super::{Plan, parse, plan};
use crate::ui_tests::{fixture, harness, wait};

const SLUG_LINK: &str = "https://www.figma.com/design/KEY/My-Design?node-id=1-5&t=abc-4";

fn figma(name: &str, node: &str) -> String {
    format!("https://www.figma.com/design/KEY/{name}?node-id={node}")
}

#[test]
fn parses_links_and_bare_node_ids() {
    let link = parse(SLUG_LINK).unwrap();
    assert_eq!(link.node, Some(Guid(1, 5)));
    assert!(link.file.unwrap().matches("My Design"));
    let bare = parse(" 1:25 ").unwrap();
    assert_eq!((bare.file, bare.node), (None, Some(Guid(1, 25))));
    assert_eq!(parse("I1:38;1:35").unwrap().node, Some(Guid(1, 38)));
    assert!(
        parse("figstudio://structure?node-id=1-4")
            .unwrap()
            .file
            .is_some()
    );
    assert!(parse("not a link").is_err());
}

#[test]
fn plan_prefers_the_active_matching_tab_then_any_then_a_local_copy() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("My Design.fig");
    std::fs::write(&copy, b"").unwrap();
    let dirs = vec![dir.path().to_owned()];
    let a = PathBuf::from("/x/My Design.fig");
    let b = PathBuf::from("/y/Other.fig");
    let c = PathBuf::from("/z/My Design (1).fig");
    let hint = parse(SLUG_LINK).unwrap().file.unwrap();
    let paths: Vec<&Path> = vec![&b, &a, &c];
    assert_eq!(plan(&paths, 2, Some(&hint), &dirs), Ok(Plan::Tab(2)));
    assert_eq!(plan(&paths, 0, Some(&hint), &dirs), Ok(Plan::Tab(1)));
    assert_eq!(plan(&[&b], 0, Some(&hint), &dirs), Ok(Plan::Open(copy)));
    let missing = plan(&[&b], 0, Some(&hint), &[]).unwrap_err();
    assert!(missing.contains("My-Design"), "{missing}");
    // No file in the link: the active tab, or an error with no tab.
    assert_eq!(plan(&paths, 1, None, &dirs), Ok(Plan::Tab(1)));
    assert!(plan(&[], 0, None, &dirs).is_err());
}

#[test]
fn a_link_to_an_open_tab_selects_and_fits_its_node() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    assert!(
        h.state_mut()
            .go_to_link(&ctx, &figma("structure", "1-25"), None)
    );
    let tab = &h.state().tabs[0];
    assert_eq!(tab.selection, Some(Guid(1, 25)));
    assert_eq!(tab.fit_node, Some(Guid(1, 25)));
    // A node the file does not have is refused, and nothing is opened.
    assert!(
        !h.state_mut()
            .go_to_link(&ctx, &figma("structure", "999-1"), None)
    );
    assert_eq!(h.state().tabs.len(), 1);
    assert!(h.state().toasts.last().unwrap().error);
}

#[test]
fn a_link_to_a_local_copy_opens_it_then_shows_the_node_and_answers() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(fixture("sections.fig"), dir.path().join("My Design.fig")).unwrap();
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.dirs = vec![dir.path().to_owned()];
    let (tx, rx) = std::sync::mpsc::channel();
    let done: super::Done = Box::new(move |result| tx.send(result).unwrap());
    assert!(h.state_mut().go_to_link(&ctx, SLUG_LINK, Some(done)));
    wait(&mut h, &mut Vec::new(), "the local copy to load", |a| {
        a.tabs.len() == 2
    });
    let app = h.state();
    assert_eq!(app.active, 1);
    assert_eq!(app.tabs[1].selection, Some(Guid(1, 5)));
    // The answer goes out at the end of the frame that loaded the file.
    let answer = rx.try_recv().unwrap().unwrap();
    assert_eq!(answer, "Showing node 1:5 in My Design.fig");
    // The same link again finds the tab now open: no second copy.
    let ctx = h.ctx.clone();
    h.state_mut().active = 0;
    assert!(h.state_mut().go_to_link(&ctx, SLUG_LINK, None));
    assert_eq!((h.state().tabs.len(), h.state().active), (2, 1));
}

#[test]
fn the_window_goes_to_the_link_typed_into_it() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().apply(crate::menu::Action::OpenLink, &ctx);
    h.step();
    // Found by its hint: the layers panel has its own search field.
    h.get_by(|n| {
        n.role() == egui::accesskit::Role::TextInput
            && n.placeholder()
                .is_some_and(|p| p.starts_with("https://www.figma.com"))
    })
    .type_text(&figma("structure", "1-4"));
    h.step();
    h.get_by_label("Go").click();
    h.step();
    h.step();
    let app = h.state();
    assert_eq!(app.tabs[0].selection, Some(Guid(1, 4)));
    assert!(!app.link.window, "the window closes once the link is shown");
    assert!(app.link.text.is_empty());
}

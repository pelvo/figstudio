//! Open Figma link: which tab or local copy a link picks (never a guess between namesakes),
//! the app showing its node, and MCP answers that carry no paths or file names.
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use figstudio_fig::Guid;

use super::{Choice, Done, LinkError, Target, parse, plan};
use crate::ui_tests::{fixture, harness, wait};

const SLUG_LINK: &str = "https://www.figma.com/design/KEY/My-Design?node-id=1-5&t=abc-4";

fn figma(name: &str, node: &str) -> String {
    format!("https://www.figma.com/design/KEY/{name}?node-id={node}")
}

/// A `Done` and the receiver of its answer.
fn done() -> (Done, mpsc::Receiver<Result<String, String>>) {
    let (tx, rx) = mpsc::channel();
    (Box::new(move |result| tx.send(result).unwrap()), rx)
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
    assert_eq!(parse("not a link"), Err(LinkError::Unreadable));
}

#[test]
fn plan_picks_the_one_best_file_among_tabs_and_local_copies() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("My Design.fig");
    std::fs::write(&copy, b"").unwrap();
    let dirs = vec![dir.path().to_owned()];
    let other = PathBuf::from("/y/Other.fig");
    let hint = parse(SLUG_LINK).unwrap().file.unwrap();
    // Only the local copy matches.
    let tabs: Vec<(u64, &Path)> = vec![(1, &other)];
    assert_eq!(
        plan(&tabs, Some(1), Some(&hint), &dirs),
        Ok(Target::Open(copy.clone()))
    );
    // The copy is open as a tab: it counts once, as the tab.
    let tabs: Vec<(u64, &Path)> = vec![(1, &other), (2, &copy)];
    assert_eq!(plan(&tabs, Some(1), Some(&hint), &dirs), Ok(Target::Tab(2)));
    // No file in the link: the active tab, or an error with no tab.
    assert_eq!(plan(&tabs, Some(1), None, &dirs), Ok(Target::Tab(1)));
    assert_eq!(plan(&[], None, None, &dirs), Err(LinkError::NoFile));
    let missing = plan(&[], None, Some(&hint), &[]).unwrap_err();
    assert!(matches!(missing, LinkError::NotFound { .. }), "{missing:?}");
}

#[test]
fn namesakes_tie_and_are_listed_but_a_file_key_wins() {
    let dir = tempfile::tempdir().unwrap();
    let newer = dir.path().join("My Design (1).fig");
    std::fs::write(&newer, b"").unwrap();
    let dirs = vec![dir.path().to_owned()];
    let open = PathBuf::from("/x/My Design.fig");
    let tabs: Vec<(u64, &Path)> = vec![(4, &open)];
    let hint = parse(SLUG_LINK).unwrap().file.unwrap();
    // An open tab and a newer download with the same name: no guess by tab or by date.
    assert_eq!(
        plan(&tabs, Some(4), Some(&hint), &dirs),
        Err(LinkError::Several(vec![
            Choice {
                target: Target::Tab(4),
                path: open.clone()
            },
            Choice {
                target: Target::Open(newer.clone()),
                path: newer
            },
        ]))
    );
    // A file named after the link's key is that file, whatever else shares the name.
    let keyed = dir.path().join("KEY.fig");
    std::fs::write(&keyed, b"").unwrap();
    assert_eq!(
        plan(&tabs, Some(4), Some(&hint), &dirs),
        Ok(Target::Open(keyed))
    );
}

#[test]
fn public_messages_carry_no_paths_or_file_names() {
    let secret = "Secret Client Brief";
    let folder = PathBuf::from("/home/example/Private Folder");
    let path = folder.join(format!("{secret}.fig"));
    let errors = [
        LinkError::NotFound {
            name: secret.to_owned(),
            dirs: vec![folder.clone()],
        },
        LinkError::Several(vec![Choice {
            target: Target::Open(path.clone()),
            path,
        }]),
        LinkError::NodeMissing {
            node: Guid(1, 2),
            title: format!("{secret}.fig"),
        },
        LinkError::LoadFailed,
        LinkError::TabClosed,
        LinkError::NoFile,
        LinkError::Unreadable,
    ];
    for error in &errors {
        let text = error.public();
        assert!(
            !text.contains(secret) && !text.contains("Private") && !text.contains("/home"),
            "{text}"
        );
    }
    // The person at the computer still gets the details.
    assert!(errors[0].local().contains("Private Folder"));
    assert!(errors[2].local().contains(secret));
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
fn a_link_to_a_local_copy_opens_it_then_shows_the_node_and_answers_without_names() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(fixture("sections.fig"), dir.path().join("My Design.fig")).unwrap();
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.dirs = vec![dir.path().to_owned()];
    let (done, rx) = done();
    assert!(h.state_mut().go_to_link(&ctx, SLUG_LINK, Some(done)));
    wait(&mut h, &mut Vec::new(), "the local copy to load", |a| {
        a.tabs.len() == 2
    });
    let app = h.state();
    assert_eq!(app.active, 1);
    assert_eq!(app.tabs[1].selection, Some(Guid(1, 5)));
    // The answer goes out at the end of the frame that loaded the file.
    let answer = rx.try_recv().unwrap().unwrap();
    let id = app.tabs[1].id;
    assert_eq!(
        answer,
        format!("revealed 1:5 in tab {id} (opened the local copy)")
    );
    // The same link again finds the tab now open: no second copy.
    let ctx = h.ctx.clone();
    h.state_mut().active = 0;
    assert!(h.state_mut().go_to_link(&ctx, SLUG_LINK, None));
    assert_eq!((h.state().tabs.len(), h.state().active), (2, 1));
}

#[test]
fn an_mcp_request_for_namesakes_fails_without_names_and_opens_nothing() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["My Design.fig", "My Design (1).fig"] {
        std::fs::copy(fixture("sections.fig"), dir.path().join(name)).unwrap();
    }
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.dirs = vec![dir.path().to_owned()];
    let (done, rx) = done();
    assert!(!h.state_mut().go_to_link(&ctx, SLUG_LINK, Some(done)));
    for (done, result) in h.state_mut().take_link_answers() {
        done(result);
    }
    let error = rx.try_recv().unwrap().unwrap_err();
    assert!(
        error.starts_with("several files match this link"),
        "{error}"
    );
    assert!(!error.contains("My Design"), "{error}");
    assert!(
        h.state().link.choices.is_empty(),
        "MCP does not fill the window"
    );
    assert_eq!(h.state().tabs.len(), 1);
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

#[test]
fn the_window_lists_namesakes_and_opens_the_one_picked() {
    use egui_kittest::kittest::Queryable;
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(fixture("sections.fig"), dir.path().join("My Design.fig")).unwrap();
    std::fs::copy(
        fixture("sections.fig"),
        dir.path().join("My Design (1).fig"),
    )
    .unwrap();
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.dirs = vec![dir.path().to_owned()];
    h.state_mut().apply(crate::menu::Action::OpenLink, &ctx);
    h.state_mut().link.text = SLUG_LINK.to_owned();
    h.step();
    h.get_by_label("Go").click();
    h.step();
    h.step();
    assert_eq!(h.state().link.choices.len(), 2);
    assert!(h.state().link.window, "the window stays open to pick");
    assert_eq!(h.state().tabs.len(), 1, "nothing opened yet");
    h.get_by_label("My Design (1).fig").click();
    h.step();
    wait(&mut h, &mut Vec::new(), "the picked copy to load", |a| {
        a.tabs.len() == 2
    });
    let app = h.state();
    assert!(app.tabs[1].path.ends_with("My Design (1).fig"));
    assert_eq!(app.tabs[1].selection, Some(Guid(1, 5)));
    assert!(!app.link.window && app.link.choices.is_empty());
}

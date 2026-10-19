//! Open Figma link: which tab or local copy a link picks (never a guess between namesakes),
//! the app showing its node, and MCP answers that carry no paths or file names.
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use figstudio_fig::Guid;

use egui_kittest::Harness;
use figstudio_mcp_core::link::{FileHint, Match};
use figstudio_mcp_core::local_copy::local_copies;

use super::{Choice, Done, LinkError, Target, as_tab_paths, parse, plan};
use crate::App;
use crate::ui_tests::{fixture, harness, wait};

const SLUG_LINK: &str = "https://www.figma.com/design/KEY/My-Design?node-id=1-5&t=abc-4";

fn figma(name: &str, node: &str) -> String {
    format!("https://www.figma.com/design/KEY/{name}?node-id={node}")
}

/// Steps until no folder search is running (they run on threads).
fn searched(h: &mut Harness<'_, App>) {
    wait(h, &mut Vec::new(), "the folder search", |a| {
        a.link.searching() == 0
    });
}

/// `plan` with the copies `local_copies` finds in `dirs`, as the search thread hands them over.
fn plan_in(
    tabs: &[(u64, &Path)],
    active: Option<u64>,
    file: Option<&FileHint>,
    dirs: &[PathBuf],
) -> Result<Target, LinkError> {
    let open: Vec<PathBuf> = tabs.iter().map(|(_, p)| p.to_path_buf()).collect();
    let copies = file.map_or_else(Vec::new, |hint| {
        as_tab_paths(local_copies(dirs, hint), &open)
    });
    plan(tabs, active, file, copies, dirs)
}

/// A listing that is slow for the file `Slow` (and then finds the `sections` fixture) and
/// finds nothing at once for any other file.
fn slow_for_one(_: &[PathBuf], hint: &FileHint) -> Vec<(PathBuf, Match)> {
    if hint.name() != "Slow" {
        return Vec::new();
    }
    std::thread::sleep(std::time::Duration::from_millis(800));
    vec![(fixture("sections.fig"), Match::Name)]
}

/// The Open Figma link window's text field.
fn link_field<'h>(h: &'h Harness<'_, App>) -> egui_kittest::Node<'h> {
    use egui_kittest::kittest::Queryable;
    h.get_by(|n| {
        n.role() == egui::accesskit::Role::TextInput
            && n.placeholder()
                .is_some_and(|p| p.starts_with("https://www.figma.com"))
    })
}

/// A folder listing that hangs, as one behind a macOS permission prompt does.
fn slow_search(_: &[PathBuf], _: &FileHint) -> Vec<(PathBuf, Match)> {
    std::thread::sleep(std::time::Duration::from_secs(2));
    Vec::new()
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
        plan_in(&tabs, Some(1), Some(&hint), &dirs),
        Ok(Target::Open(copy.clone()))
    );
    // The copy is open as a tab: it counts once, as the tab.
    let tabs: Vec<(u64, &Path)> = vec![(1, &other), (2, &copy)];
    assert_eq!(
        plan_in(&tabs, Some(1), Some(&hint), &dirs),
        Ok(Target::Tab(2))
    );
    // No file in the link: the active tab, or an error with no tab.
    assert_eq!(plan_in(&tabs, Some(1), None, &dirs), Ok(Target::Tab(1)));
    assert_eq!(plan_in(&[], None, None, &dirs), Err(LinkError::NoFile));
    let missing = plan_in(&[], None, Some(&hint), &[]).unwrap_err();
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
        plan_in(&tabs, Some(4), Some(&hint), &dirs),
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
        plan_in(&tabs, Some(4), Some(&hint), &dirs),
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
        LinkError::SearchTimeout,
    ];
    for error in &errors {
        let text = error.public();
        assert!(
            !text.contains(secret) && !text.contains("Private") && !text.contains("/home"),
            "{text}"
        );
    }
    // The permission hint is for macOS only; elsewhere a hung folder has other causes.
    let timeout = LinkError::SearchTimeout.public();
    assert_eq!(
        timeout.contains("permission"),
        cfg!(target_os = "macos"),
        "{timeout}"
    );
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
    searched(&mut h);
    let tab = &h.state().tabs[0];
    // Selected; the canvas has already taken `fit_node` to fit it.
    assert_eq!(tab.selection, Some(Guid(1, 25)));
    // A node the file does not have is refused, and nothing is opened.
    assert!(
        h.state_mut()
            .go_to_link(&ctx, &figma("structure", "999-1"), None)
    );
    searched(&mut h);
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
    searched(&mut h);
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
    assert!(h.state_mut().go_to_link(&ctx, SLUG_LINK, Some(done)));
    searched(&mut h);
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
    searched(&mut h);
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
    searched(&mut h);
    h.step();
    assert_eq!(h.state().link.choices.len(), 2);
    assert!(h.state().link.window, "the window shows the files to pick");
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

#[test]
fn a_slow_folder_does_not_block_the_window_and_the_link_fails_with_a_reason() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = slow_search;
    h.state_mut().link.search_timeout = std::time::Duration::from_millis(200);
    let (done, rx) = done();
    let started = std::time::Instant::now();
    assert!(h.state_mut().go_to_link(&ctx, SLUG_LINK, Some(done)));
    assert!(
        started.elapsed() < std::time::Duration::from_millis(500),
        "go_to_link waited for the folders: {:?}",
        started.elapsed()
    );
    // Frames keep running while the folder hangs; the link fails at the timeout.
    searched(&mut h);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    h.step();
    let error = rx.try_recv().unwrap().unwrap_err();
    assert!(error.contains("could not be read in time"), "{error}");
    assert!(error.contains("permission"), "{error}");
    // The late result is dropped, not answered twice or shown.
    std::thread::sleep(std::time::Duration::from_secs(2));
    h.step();
    assert_eq!(h.state().tabs.len(), 1);
}

#[test]
fn a_new_window_link_replaces_the_older_search_whose_late_result_is_dropped() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = slow_for_one;
    h.state_mut().apply(crate::menu::Action::OpenLink, &ctx);
    h.step();
    link_field(&h).type_text(&figma("Slow", "0-1"));
    h.step();
    h.get_by_label("Go").click();
    h.step();
    // Still searching for the first link: the second one replaces it.
    assert_eq!(h.state().link.searching(), 1);
    h.state_mut().link.text = figma("structure", "1-4");
    h.step();
    h.get_by_label("Go").click();
    h.step();
    searched(&mut h);
    assert_eq!(h.state().tabs[0].selection, Some(Guid(1, 4)));
    // The first listing comes back later: nothing opens and the window stays closed.
    std::thread::sleep(std::time::Duration::from_millis(1200));
    for _ in 0..3 {
        h.step();
    }
    let app = h.state();
    assert_eq!(app.tabs.len(), 1, "the replaced search opened its file");
    assert!(!app.link.window);
}

#[test]
fn while_a_listing_is_blocked_past_its_time_new_links_fail_at_once() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = slow_search;
    h.state_mut().link.search_timeout = std::time::Duration::from_millis(200);
    let (first, first_rx) = done();
    assert!(h.state_mut().go_to_link(&ctx, SLUG_LINK, Some(first)));
    searched(&mut h);
    assert!(first_rx.try_recv().unwrap().is_err());
    // The listing is still blocked: no second thread, an answer at once.
    let (second, second_rx) = done();
    let started = std::time::Instant::now();
    assert!(!h.state_mut().go_to_link(&ctx, SLUG_LINK, Some(second)));
    assert_eq!(h.state().link.searching(), 0);
    h.step();
    let error = second_rx.try_recv().unwrap().unwrap_err();
    assert!(error.contains("could not be read in time"), "{error}");
    assert!(started.elapsed() < std::time::Duration::from_millis(150));
    // Once the blocked listing returns, links search again.
    std::thread::sleep(std::time::Duration::from_secs(2));
    h.step();
    assert!(h.state_mut().go_to_link(&ctx, SLUG_LINK, None));
    assert_eq!(h.state().link.searching(), 1);
}

#[test]
fn starting_a_search_schedules_the_frame_that_watches_it() {
    // No file open: nothing else asks for frames, as in an empty window waiting on a folder.
    let mut h = Harness::builder()
        .with_size(egui::vec2(800.0, 600.0))
        .build_eframe(|cc| App::new(cc, Vec::new()));
    let ctx = h.ctx.clone();
    h.state_mut().link.search = slow_search;
    for _ in 0..50 {
        if !ctx.has_requested_repaint() {
            break;
        }
        h.step();
    }
    assert!(!ctx.has_requested_repaint(), "the empty app is not idle");
    assert!(h.state_mut().go_to_link(&ctx, SLUG_LINK, None));
    assert!(
        ctx.has_requested_repaint(),
        "no frame would poll the search, so its timeout would never fire"
    );
}

#[test]
fn the_window_keeps_its_link_while_searching_and_after_a_miss() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = slow_search;
    h.state_mut().apply(crate::menu::Action::OpenLink, &ctx);
    h.step();
    link_field(&h).type_text(&figma("Missing", "1-4"));
    h.step();
    h.get_by_label("Go").click();
    h.step();
    h.step();
    assert!(h.state().link.window, "the window closed while searching");
    assert_eq!(h.state().link.text, figma("Missing", "1-4"));
    h.get_by_label("Searching the link folders\u{2026}");
    // Nothing is found: the window and its text stay for another try.
    searched(&mut h);
    h.step();
    let app = h.state();
    assert!(app.link.window, "the window closed on a miss");
    assert_eq!(app.link.text, figma("Missing", "1-4"));
}

#[test]
fn a_result_that_comes_after_the_timeout_is_not_used() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = slow_for_one;
    h.state_mut().link.search_timeout = std::time::Duration::from_millis(200);
    let (done, rx) = done();
    assert!(
        h.state_mut()
            .go_to_link(&ctx, &figma("Slow", "0-1"), Some(done))
    );
    // No frame runs until the listing is back, well past the timeout.
    std::thread::sleep(std::time::Duration::from_millis(1200));
    h.step();
    h.step();
    let error = rx.try_recv().unwrap().unwrap_err();
    assert!(error.contains("could not be read in time"), "{error}");
    assert_eq!(h.state().tabs.len(), 1, "the late result was opened");
}

#[test]
fn a_copy_that_is_an_open_tab_is_found_by_its_real_path_off_the_ui_thread() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let tab = dir.path().join("My Design.fig");
    std::fs::write(&tab, b"").unwrap();
    let listed = dir.path().join("sub/../My Design.fig");
    let copies = as_tab_paths(
        vec![(listed.clone(), Match::Name)],
        std::slice::from_ref(&tab),
    );
    assert_eq!(copies, vec![(tab.clone(), Match::Name)]);
    // So plan, which does no I/O, sees the tab and not a second file.
    let hint = parse(SLUG_LINK).unwrap().file.unwrap();
    let tabs = [(9, tab.as_path())];
    assert_eq!(
        plan(&tabs, Some(9), Some(&hint), copies, &[]),
        Ok(Target::Tab(9))
    );
    // A file that is not open keeps its listed path.
    let other = as_tab_paths(vec![(listed.clone(), Match::Name)], &[]);
    assert_eq!(other, vec![(listed, Match::Name)]);
}

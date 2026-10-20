//! Open Figma link: which tab or local copy a link picks (never a guess between namesakes),
//! the app showing its node, and MCP answers that carry no paths or file names.
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use figstudio_fig::Guid;

use egui_kittest::Harness;
use figstudio_mcp_core::link::{FileHint, Match};
use figstudio_mcp_core::local_copy::local_copies;

use super::{Choice, Done, LinkError, Target, parse, plan, real_paths};
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

/// `plan` with the copies `local_copies` finds in `dirs`, as the search thread hands them over,
/// and tabs whose loader resolved their paths.
fn plan_in(
    tabs: &[(u64, &Path)],
    active: Option<u64>,
    file: Option<&FileHint>,
    dirs: &[PathBuf],
) -> Result<Target, LinkError> {
    let real: Vec<PathBuf> = tabs
        .iter()
        .map(|(_, p)| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
        .collect();
    let tabs: Vec<(u64, &Path, &Path)> = tabs
        .iter()
        .zip(&real)
        .map(|(&(id, path), real)| (id, path, real.as_path()))
        .collect();
    let copies = file.map_or_else(Vec::new, |hint| real_paths(local_copies(dirs, hint)));
    plan(&tabs, active, file, copies, dirs)
}

/// After 800 ms, the `structure` fixture under a path spelled differently from the usual one
/// (`fixtures/../fixtures/structure.fig`; `Path` equality skips `.` but not `..`).
fn slow_structure(_: &[PathBuf], _: &FileHint) -> Vec<(PathBuf, Match)> {
    std::thread::sleep(std::time::Duration::from_millis(800));
    let path = fixture("../fixtures/structure.fig");
    assert_ne!(path, fixture("structure.fig"));
    vec![(path, Match::Name)]
}

/// At once, the `structure` fixture spelled `fixtures/../fixtures/structure.fig`.
fn listed_structure(_: &[PathBuf], _: &FileHint) -> Vec<(PathBuf, Match)> {
    vec![(fixture("../fixtures/structure.fig"), Match::Name)]
}

/// An app with no file open.
fn empty_app() -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::vec2(800.0, 600.0))
        .build_eframe(|cc| App::new(cc, Vec::new()))
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
        LinkError::ListingLimit,
        LinkError::StillLoading,
    ];
    for error in &errors {
        let text = error.public();
        assert!(
            !text.contains(secret) && !text.contains("Private") && !text.contains("/home"),
            "{text}"
        );
    }
    // Both kinds of stuck reads may be waiting on a macOS permission dialog.
    for error in [LinkError::SearchTimeout, LinkError::ListingLimit] {
        let text = error.public();
        assert_eq!(
            text.contains("permission"),
            cfg!(target_os = "macos"),
            "{text}"
        );
    }
    let limit = LinkError::ListingLimit.public();
    assert!(limit.contains("wait for them to finish"), "{limit}");
    assert!(limit.contains("restart figstudio"), "{limit}");
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
    // The permission hint is for macOS only.
    assert_eq!(
        error.contains("permission"),
        cfg!(target_os = "macos"),
        "{error}"
    );
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
    let mut h = empty_app();
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
fn a_tab_opened_while_the_folders_are_listed_is_not_opened_again() {
    let mut h = empty_app();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = slow_structure;
    let (done, rx) = done();
    assert!(
        h.state_mut()
            .go_to_link(&ctx, &figma("structure", "1-4"), Some(done))
    );
    // While the listing runs, the same file is opened under its usual path.
    h.state_mut().open(fixture("structure.fig"));
    wait(&mut h, &mut Vec::new(), "the file to load", |a| {
        a.tabs.len() == 1
    });
    searched(&mut h);
    h.step();
    let answer = rx.try_recv().unwrap().unwrap();
    assert!(!answer.contains("opened"), "{answer}");
    let app = h.state();
    assert_eq!(app.tabs.len(), 1, "the file was loaded a second time");
    assert_eq!(app.tabs[0].selection, Some(Guid(1, 4)));
}

#[test]
fn a_node_id_from_the_window_replaces_its_running_search() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = slow_for_one;
    assert!(h.state_mut().go_to_link(&ctx, &figma("Slow", "0-1"), None));
    assert!(h.state_mut().go_to_link(&ctx, "1:4", None));
    assert_eq!(h.state().link.searching(), 0, "the older search still runs");
    assert_eq!(h.state().tabs[0].selection, Some(Guid(1, 4)));
    std::thread::sleep(std::time::Duration::from_millis(1200));
    for _ in 0..3 {
        h.step();
    }
    let app = h.state();
    assert_eq!(app.tabs.len(), 1, "the replaced search opened its file");
    assert_eq!(app.tabs[0].selection, Some(Guid(1, 4)));
}

#[test]
fn a_refused_link_leaves_the_window_search_running_and_a_repeat_starts_no_thread() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = slow_search;
    h.state_mut().link.search_timeout = std::time::Duration::from_millis(600);
    assert!(h.state_mut().go_to_link(&ctx, &figma("A", "1-1"), None));
    std::thread::sleep(std::time::Duration::from_millis(300));
    // B replaces A, whose listing stays blocked; Go on B again starts nothing new.
    assert!(h.state_mut().go_to_link(&ctx, &figma("B", "1-1"), None));
    assert!(h.state_mut().go_to_link(&ctx, &figma("B", "1-1"), None));
    assert_eq!(
        (h.state().link.searching(), h.state().link.listings.len()),
        (1, 2)
    );
    // A's listing is now past the timeout; B's search is not. C is refused, B stays.
    std::thread::sleep(std::time::Duration::from_millis(400));
    assert!(!h.state_mut().go_to_link(&ctx, &figma("C", "1-1"), None));
    assert_eq!(
        h.state().link.searching(),
        1,
        "the refused link dropped the running search"
    );
}

#[test]
fn closing_the_window_cancels_its_search() {
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
    assert_eq!(h.state().link.searching(), 1);
    h.get_by_label("Close window").click();
    h.step();
    assert_eq!(
        h.state().link.searching(),
        0,
        "the search outlived its window"
    );
    std::thread::sleep(std::time::Duration::from_millis(1200));
    for _ in 0..3 {
        h.step();
    }
    let app = h.state();
    assert_eq!(
        app.tabs.len(),
        1,
        "the closed window's search opened its file"
    );
    assert!(!app.link.window);
}

#[test]
fn a_listing_waits_for_a_file_still_loading_before_it_opens_anything() {
    let mut h = empty_app();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = listed_structure;
    // The person has just opened the file: its load is in flight, it is no tab yet.
    let opened = fixture("structure.fig");
    assert!(h.state_mut().loading.begin(&opened));
    let (done, rx) = done();
    assert!(
        h.state_mut()
            .go_to_link(&ctx, &figma("structure", "1-4"), Some(done))
    );
    std::thread::sleep(std::time::Duration::from_millis(200));
    for _ in 0..5 {
        h.step();
    }
    assert_eq!(
        h.state().link.searching(),
        1,
        "the listing was used during the load"
    );
    assert_eq!(h.state().loading.iter().count(), 1, "a second load started");
    assert!(rx.try_recv().is_err());
    // The load runs and ends; the link then finds the new tab.
    h.state_mut().loading.finish(&opened);
    h.state_mut().open(opened);
    searched(&mut h);
    h.step();
    let answer = rx.try_recv().unwrap().unwrap();
    assert!(!answer.contains("opened"), "{answer}");
    let app = h.state();
    assert_eq!(app.tabs.len(), 1, "the file was loaded a second time");
    assert_eq!(app.tabs[0].selection, Some(Guid(1, 4)));
}

#[test]
fn a_node_id_the_file_lacks_still_replaces_the_window_search() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = slow_for_one;
    assert!(h.state_mut().go_to_link(&ctx, &figma("Slow", "0-1"), None));
    assert!(!h.state_mut().go_to_link(&ctx, "999:1", None));
    assert_eq!(h.state().link.searching(), 0, "the older search still runs");
}

#[test]
fn a_file_loading_that_the_link_cannot_name_does_not_hold_it() {
    let mut h = empty_app();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = listed_structure;
    assert!(
        h.state_mut()
            .loading
            .begin(&PathBuf::from("/elsewhere/Other.fig"))
    );
    let (done, rx) = done();
    assert!(
        h.state_mut()
            .go_to_link(&ctx, &figma("structure", "1-4"), Some(done))
    );
    // The copy opens while the unrelated load is still in flight.
    wait(&mut h, &mut Vec::new(), "the copy to open", |a| {
        a.tabs.len() == 1
    });
    searched(&mut h);
    h.step();
    assert!(rx.try_recv().unwrap().unwrap().contains("opened"));
}

#[test]
fn a_listing_held_for_a_load_that_never_ends_fails_at_the_timeout() {
    let mut h = empty_app();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = listed_structure;
    h.state_mut().link.search_timeout = std::time::Duration::from_millis(300);
    assert!(h.state_mut().loading.begin(&fixture("structure.fig")));
    let (done, rx) = done();
    let started = std::time::Instant::now();
    assert!(
        h.state_mut()
            .go_to_link(&ctx, &figma("structure", "1-4"), Some(done))
    );
    searched(&mut h);
    h.step();
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    let error = rx.try_recv().unwrap().unwrap_err();
    assert!(error.contains("still opening"), "{error}");
    assert!(h.state().tabs.is_empty());
}

#[test]
fn a_copy_whose_load_just_failed_is_not_loaded_again() {
    let mut h = empty_app();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = listed_structure;
    let opened = fixture("structure.fig");
    assert!(h.state_mut().loading.begin(&opened));
    let (done, rx) = done();
    assert!(
        h.state_mut()
            .go_to_link(&ctx, &figma("structure", "1-4"), Some(done))
    );
    std::thread::sleep(std::time::Duration::from_millis(200));
    h.step();
    // The load fails, as `poll` reports a `Loaded::Err`.
    let real = std::fs::canonicalize(&opened).unwrap();
    h.state_mut().loading.finish(&opened);
    h.state_mut().link_failed(&opened, Some(real));
    h.step();
    let error = rx.try_recv().unwrap().unwrap_err();
    assert!(error.contains("could not be opened"), "{error}");
    let app = h.state();
    assert_eq!(
        app.loading.iter().count(),
        0,
        "the failed file was loaded again"
    );
    assert!(app.tabs.is_empty());
}

/// The figma.com key's own file in `dir` (`KEY.fig`, listed as `dir/sub/../KEY.fig` with no
/// `sub`, so it cannot be resolved), then the `structure` fixture, a name match only.
fn key_then_name(dir: &Path) -> Vec<(PathBuf, Match)> {
    vec![
        (dir.join("sub/../KEY.fig"), Match::Key),
        (fixture("structure.fig"), Match::Name),
    ]
}

fn key_dir() -> &'static Path {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        std::fs::copy(fixture("structure.fig"), dir.path().join("KEY.fig")).unwrap();
        dir
    })
    .path()
}

fn key_listing(_: &[PathBuf], _: &FileHint) -> Vec<(PathBuf, Match)> {
    key_then_name(key_dir())
}

#[test]
fn a_failed_best_match_fails_the_link_instead_of_opening_a_worse_one() {
    let mut h = empty_app();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = key_listing;
    // The key's file is loading under its plain spelling; the worker cannot resolve it.
    let opened = key_dir().join("KEY.fig");
    assert!(h.state_mut().loading.begin(&opened));
    let (done, rx) = done();
    assert!(
        h.state_mut()
            .go_to_link(&ctx, &figma("structure", "1-4"), Some(done))
    );
    std::thread::sleep(std::time::Duration::from_millis(200));
    h.step();
    h.state_mut().loading.finish(&opened);
    h.state_mut().link_failed(&opened, None);
    h.step();
    let error = rx.try_recv().unwrap().unwrap_err();
    assert!(error.contains("could not be opened"), "{error}");
    let app = h.state();
    assert_eq!(
        app.loading.iter().count(),
        0,
        "a file was loaded in its place"
    );
    assert!(app.tabs.is_empty());
}

#[test]
fn a_listing_stuck_in_old_folders_does_not_block_links_to_new_ones() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = slow_search;
    h.state_mut().link.search_timeout = std::time::Duration::from_millis(200);
    assert!(h.state_mut().go_to_link(&ctx, &figma("A", "1-1"), None));
    std::thread::sleep(std::time::Duration::from_millis(300));
    h.step();
    // The folders still have not answered: links are refused.
    assert!(!h.state_mut().go_to_link(&ctx, &figma("B", "1-1"), None));
    // The person chooses another folder: links search again.
    h.state_mut()
        .link
        .set_dirs(vec![PathBuf::from("/elsewhere")]);
    assert!(h.state_mut().go_to_link(&ctx, &figma("B", "1-1"), None));
    assert_eq!(h.state().link.searching(), 1);
}

#[test]
fn a_listing_that_finished_in_time_is_used_even_when_polled_late() {
    let mut h = empty_app();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = listed_structure;
    h.state_mut().link.search_timeout = std::time::Duration::from_millis(300);
    let (done, rx) = done();
    assert!(
        h.state_mut()
            .go_to_link(&ctx, &figma("structure", "1-4"), Some(done))
    );
    // The listing is back at once; the next frame runs only after the deadline.
    std::thread::sleep(std::time::Duration::from_millis(600));
    h.step();
    wait(&mut h, &mut Vec::new(), "the copy to open", |a| {
        a.tabs.len() == 1
    });
    h.step();
    let answer = rx.try_recv().unwrap().unwrap();
    assert!(answer.contains("opened"), "{answer}");
}

#[test]
fn a_load_that_outlasts_the_callers_budget_is_answered_and_still_shown() {
    let mut h = harness();
    let (done, rx) = done();
    h.state_mut().link.waiting.push(super::Waiting {
        path: PathBuf::from("/nowhere/Slow Load.fig"),
        node: Some(Guid(1, 4)),
        done: Some(done),
        asked: web_time::Instant::now(),
        answered: false,
    });
    h.step();
    assert!(rx.try_recv().is_err(), "answered before its time");
    h.state_mut().link.answer_by = std::time::Duration::ZERO;
    h.step();
    let error = rx.try_recv().unwrap().unwrap_err();
    assert!(error.contains("still opening"), "{error}");
    // Still waiting for the load, to show the node once it opens.
    assert_eq!(h.state().link.waiting.len(), 1);
}

#[cfg(feature = "mcp")]
#[test]
fn mcp_requests_are_answered_before_the_server_gives_up() {
    assert!(super::ANSWER_BY < figstudio_mcp::live::OPEN_TIMEOUT);
}

#[test]
fn choosing_the_same_stuck_folder_again_lets_links_search_again() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = slow_search;
    h.state_mut().link.search_timeout = std::time::Duration::from_millis(200);
    assert!(h.state_mut().go_to_link(&ctx, &figma("A", "1-1"), None));
    std::thread::sleep(std::time::Duration::from_millis(300));
    h.step();
    assert!(!h.state_mut().go_to_link(&ctx, &figma("B", "1-1"), None));
    // The only folder (a share that has woken, say) is chosen again: a fresh search starts.
    let dirs = h.state().link.dirs.clone();
    h.state_mut().link.set_dirs(dirs);
    assert!(h.state_mut().go_to_link(&ctx, &figma("B", "1-1"), None));
    assert_eq!(h.state().link.searching(), 1);
}

#[test]
fn retries_cannot_pile_up_blocked_threads() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = slow_search;
    h.state_mut().link.search_timeout = std::time::Duration::from_millis(300);
    let dirs = h.state().link.dirs.clone();
    for i in 0..super::MAX_ABANDONED {
        assert!(
            h.state_mut()
                .go_to_link(&ctx, &figma("A", "1-1"), Some(Box::new(|_| ()))),
            "retry {i}"
        );
        h.state_mut().link.set_dirs(dirs.clone());
    }
    // The given-up listings are past their time and still hung: no further thread starts.
    std::thread::sleep(std::time::Duration::from_millis(400));
    let (refused, rx) = done();
    assert!(
        !h.state_mut()
            .go_to_link(&ctx, &figma("A", "1-1"), Some(refused))
    );
    h.step();
    let error = rx.try_recv().unwrap().unwrap_err();
    assert_eq!(error, LinkError::ListingLimit.public());
    // Choosing a healthy folder cannot release stuck threads; the cap still explains recovery.
    h.state_mut().link.set_dirs(vec![PathBuf::from("/healthy")]);
    let (refused, rx) = done();
    assert!(
        !h.state_mut()
            .go_to_link(&ctx, &figma("A", "1-1"), Some(refused))
    );
    h.step();
    let error = rx.try_recv().unwrap().unwrap_err();
    assert_eq!(error, LinkError::ListingLimit.public());
    // Once they come back, links search again.
    std::thread::sleep(std::time::Duration::from_secs(2));
    h.step();
    assert!(h.state_mut().go_to_link(&ctx, &figma("A", "1-1"), None));
}

#[test]
fn slow_listings_given_up_on_do_not_refuse_the_newly_chosen_folder() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = slow_search;
    // Several links in flight, none past its time yet (three quick Gos, or reveal_node calls).
    for name in ["A", "B", "C", "D"] {
        assert!(
            h.state_mut()
                .go_to_link(&ctx, &figma(name, "1-1"), Some(Box::new(|_| ())))
        );
    }
    h.state_mut()
        .link
        .set_dirs(vec![PathBuf::from("/elsewhere")]);
    assert!(
        h.state_mut().go_to_link(&ctx, &figma("E", "1-1"), None),
        "the first search of the new folder was refused"
    );
}

#[test]
fn a_caller_whose_budget_runs_out_while_listing_is_answered_in_time() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = slow_search;
    h.state_mut().link.answer_by = std::time::Duration::from_secs(5);
    // Queued for 10 s before a frame took it; the folders then take 2 s to list.
    let asked = web_time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(10))
        .unwrap();
    let (done, rx) = done();
    let started = std::time::Instant::now();
    assert!(
        h.state_mut()
            .go_to_link_at(&ctx, &figma("A", "1-1"), Some(done), asked)
    );
    h.step();
    let error = rx.try_recv().unwrap().unwrap_err();
    assert!(error.contains("could not be read in time"), "{error}");
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(h.state().link.searching(), 0);
}

#[test]
fn the_callers_budget_counts_from_when_it_asked_not_when_the_app_took_it() {
    let mut h = empty_app();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = listed_structure;
    // The request waited a second before a frame took it.
    let asked = web_time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(1))
        .unwrap();
    let (done, _rx) = done();
    assert!(
        h.state_mut()
            .go_to_link_at(&ctx, &figma("structure", "1-4"), Some(done), asked)
    );
    // The frame that finishes the search starts the load; the load ends in a later frame.
    searched(&mut h);
    let waiting = &h.state().link.waiting;
    assert_eq!(waiting.len(), 1, "the copy is loading");
    assert_eq!(
        waiting[0].asked, asked,
        "the load's deadline counts from the caller's request"
    );
}

#[test]
fn lexical_folding_stacks_parents_it_cannot_remove() {
    use super::lexical;
    let fold = |p: &str| lexical(Path::new(p));
    assert_eq!(fold("dir/sub/../KEY.fig"), PathBuf::from("dir/KEY.fig"));
    assert_eq!(fold("./a/./b.fig"), PathBuf::from("a/b.fig"));
    // Parents with nothing left to remove stack up, so these stay distinct files.
    assert_eq!(fold("../../KEY.fig"), PathBuf::from("../../KEY.fig"));
    assert_eq!(fold("a/../../../KEY.fig"), PathBuf::from("../../KEY.fig"));
    assert_ne!(fold("../../KEY.fig"), fold("KEY.fig"));
    // Above a root there is nothing: `/..` is `/`.
    assert_eq!(fold("/../KEY.fig"), PathBuf::from("/KEY.fig"));
    assert_eq!(fold("/../../KEY.fig"), PathBuf::from("/KEY.fig"));
}

/// `KEY.fig` and its browser download copy `KEY (1).fig` in [`key_dir`]: both key matches.
fn key_tie(_: &[PathBuf], _: &FileHint) -> Vec<(PathBuf, Match)> {
    let dir = key_dir();
    let copy = dir.join("KEY (1).fig");
    if !copy.exists() {
        std::fs::copy(fixture("structure.fig"), &copy).unwrap();
    }
    vec![(dir.join("KEY.fig"), Match::Key), (copy, Match::Key)]
}

#[test]
fn a_key_tie_stays_a_tie_when_one_of_the_files_just_failed() {
    let mut h = empty_app();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = key_tie;
    let failed = key_dir().join("KEY.fig");
    assert!(h.state_mut().loading.begin(&failed));
    let (done, rx) = done();
    assert!(
        h.state_mut()
            .go_to_link(&ctx, &figma("structure", "1-4"), Some(done))
    );
    std::thread::sleep(std::time::Duration::from_millis(200));
    h.step();
    h.state_mut().loading.finish(&failed);
    let real = std::fs::canonicalize(&failed).unwrap();
    h.state_mut().link_failed(&failed, Some(real));
    h.step();
    let error = rx.try_recv().unwrap().unwrap_err();
    assert!(error.contains("several files match"), "{error}");
    let app = h.state();
    assert_eq!(app.loading.iter().count(), 0, "the other copy was opened");
    assert!(app.tabs.is_empty());
}

/// `KEY.fig` listed through a symlink to [`key_dir`] (as a folder reached by a link would be),
/// and a worse name match. Text folding cannot see through the link; only the copy's resolved
/// path names the same file as a failure reported by its resolved path.
#[cfg(unix)]
fn key_through_a_link(_: &[PathBuf], _: &FileHint) -> Vec<(PathBuf, Match)> {
    static LINK: std::sync::OnceLock<(tempfile::TempDir, PathBuf)> = std::sync::OnceLock::new();
    let (_, alias) = LINK.get_or_init(|| {
        let holder = tempfile::tempdir().unwrap();
        let alias = holder.path().join("alias");
        std::os::unix::fs::symlink(key_dir(), &alias).unwrap();
        (holder, alias)
    });
    vec![
        (alias.join("KEY.fig"), Match::Key),
        (fixture("structure.fig"), Match::Name),
    ]
}

#[cfg(unix)]
#[test]
fn a_failed_best_match_with_a_resolved_path_does_not_open_a_worse_one() {
    let mut h = empty_app();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = key_through_a_link;
    let opened = key_dir().join("KEY.fig");
    assert!(h.state_mut().loading.begin(&opened));
    let (done, rx) = done();
    assert!(
        h.state_mut()
            .go_to_link(&ctx, &figma("structure", "1-4"), Some(done))
    );
    std::thread::sleep(std::time::Duration::from_millis(200));
    h.step();
    h.state_mut().loading.finish(&opened);
    let real = std::fs::canonicalize(&opened).unwrap();
    h.state_mut().link_failed(&opened, Some(real));
    h.step();
    let error = rx.try_recv().unwrap().unwrap_err();
    assert!(error.contains("could not be opened"), "{error}");
    let app = h.state();
    assert_eq!(app.loading.iter().count(), 0, "the worse match was loaded");
    assert!(app.tabs.is_empty());
}

#[cfg(windows)]
#[test]
fn lexical_folding_keeps_parents_after_a_rootless_drive() {
    use super::lexical;
    let fold = |p: &str| lexical(Path::new(p));
    // `C:` alone is that drive's current folder, so its parent is somewhere else.
    assert_eq!(fold(r"C:..\KEY.fig"), PathBuf::from(r"C:..\KEY.fig"));
    assert_eq!(fold(r"C:foo\..\..\KEY.fig"), PathBuf::from(r"C:..\KEY.fig"));
    assert_ne!(fold(r"C:..\KEY.fig"), fold(r"C:KEY.fig"));
    // With a root there is nothing above it.
    assert_eq!(fold(r"C:\..\KEY.fig"), PathBuf::from(r"C:\KEY.fig"));
}

#[test]
fn a_held_search_past_the_budget_is_answered_and_still_shows_the_node() {
    use egui_kittest::kittest::Queryable;
    let mut h = empty_app();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = listed_structure;
    // The file the link names is already opening (not a tab yet), so the listing is held.
    let opened = fixture("structure.fig");
    assert!(h.state_mut().loading.begin(&opened));
    let (done, rx) = done();
    assert!(
        h.state_mut()
            .go_to_link(&ctx, &figma("structure", "1-4"), Some(done))
    );
    wait(&mut h, &mut Vec::new(), "the listing to be held", |a| {
        a.link.searches.iter().any(|s| s.found.is_some())
    });
    // The caller's budget runs out while the listing is held.
    h.state_mut().link.answer_by = std::time::Duration::ZERO;
    h.step();
    let error = rx.try_recv().unwrap().unwrap_err();
    assert!(error.contains("still opening"), "{error}");
    assert_eq!(
        h.state().link.searching(),
        1,
        "the held listing was dropped"
    );
    // Not the window's: the window would show "Searching…" and a window link would cancel it.
    assert!(!h.state().link.window_searching());
    h.state_mut().link.window = true;
    h.step();
    assert!(
        h.query_by_label("Searching the link folders\u{2026}")
            .is_none(),
        "the window shows an answered MCP search as its own"
    );
    h.state_mut().link.window = false;
    // Its own search deadline passes too: it keeps waiting for the load, quietly.
    h.state_mut().link.search_timeout = std::time::Duration::ZERO;
    h.step();
    assert_eq!(
        h.state().link.searching(),
        1,
        "the answered search was cut off"
    );
    assert!(h.state().toasts.is_empty(), "a second answer was toasted");
    // The load ends: the node is shown, and the caller is not answered twice.
    h.state_mut().loading.finish(&opened);
    h.state_mut().open(opened);
    searched(&mut h);
    h.step();
    let app = h.state();
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(
        app.tabs[0].selection,
        Some(Guid(1, 4)),
        "the node was not shown"
    );
    assert!(rx.try_recv().is_err());
    assert!(app.toasts.is_empty(), "the answered search toasted");
}

#[test]
fn a_request_past_its_budget_while_loading_is_answered_and_the_node_shown() {
    let mut h = empty_app();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = listed_structure;
    // Hold the listing (a file with the link's name is opening elsewhere) to see it in hand.
    let other = PathBuf::from("/elsewhere/structure.fig");
    assert!(h.state_mut().loading.begin(&other));
    let asked = web_time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(10))
        .unwrap();
    let (done, rx) = done();
    assert!(
        h.state_mut()
            .go_to_link_at(&ctx, &figma("structure", "1-4"), Some(done), asked)
    );
    wait(&mut h, &mut Vec::new(), "the listing to be held", |a| {
        a.link.searches.iter().any(|s| s.found.is_some())
    });
    // Release it with the caller 10 s in against a 5 s budget: the search finishes first in
    // the next frame, starts the copy's load, and that load is past the budget.
    h.state_mut().link.answer_by = std::time::Duration::from_secs(5);
    h.state_mut().loading.finish(&other);
    h.step();
    let error = rx.try_recv().unwrap().unwrap_err();
    assert!(error.contains("still opening"), "{error}");
    // The copy still opens and shows the node, with no second answer.
    wait(&mut h, &mut Vec::new(), "the copy to open", |a| {
        a.tabs.len() == 1
    });
    h.step();
    assert_eq!(h.state().tabs[0].selection, Some(Guid(1, 4)));
    assert!(rx.try_recv().is_err(), "the caller was answered twice");
}

#[test]
fn a_load_answered_early_says_nothing_more_when_it_ends() {
    let mut h = empty_app();
    let opened = fixture("structure.fig");
    let (done, rx) = done();
    // Waiting for a node the file does not have; the budget answers first.
    h.state_mut().link.waiting.push(super::Waiting {
        path: opened.clone(),
        node: Some(Guid(999, 1)),
        done: Some(done),
        asked: web_time::Instant::now(),
        answered: false,
    });
    h.state_mut().link.answer_by = std::time::Duration::ZERO;
    h.step();
    assert!(
        rx.try_recv()
            .unwrap()
            .unwrap_err()
            .contains("still opening")
    );
    h.state_mut().open(opened);
    wait(&mut h, &mut Vec::new(), "the file to open", |a| {
        a.tabs.len() == 1
    });
    h.step();
    assert!(
        h.state().toasts.is_empty(),
        "an answered load toasted its outcome"
    );
}

#[test]
fn an_answered_held_search_opens_its_copy_quietly() {
    let mut h = empty_app();
    let ctx = h.ctx.clone();
    h.state_mut().link.search = listed_structure;
    let other = PathBuf::from("/elsewhere/structure.fig");
    assert!(h.state_mut().loading.begin(&other));
    let (done, rx) = done();
    assert!(
        h.state_mut()
            .go_to_link(&ctx, &figma("structure", "1-4"), Some(done))
    );
    wait(&mut h, &mut Vec::new(), "the listing to be held", |a| {
        a.link.searches.iter().any(|s| s.found.is_some())
    });
    h.state_mut().link.answer_by = std::time::Duration::ZERO;
    h.step();
    assert!(
        rx.try_recv()
            .unwrap()
            .unwrap_err()
            .contains("still opening")
    );
    // Released: the copy opens and shows the node, with no "Opening…" toast or answer.
    h.state_mut().loading.finish(&other);
    wait(&mut h, &mut Vec::new(), "the copy to open", |a| {
        a.tabs.len() == 1
    });
    h.step();
    let app = h.state();
    assert_eq!(app.tabs[0].selection, Some(Guid(1, 4)));
    assert!(app.toasts.is_empty(), "the answered search toasted");
    assert!(rx.try_recv().is_err());
}

/// Get a real MCP search answered while its listing is held for a same-named load.
fn answered_held_search(
    h: &mut Harness<'_, App>,
    text: &str,
    search: super::SearchFn,
) -> (PathBuf, mpsc::Receiver<Result<String, String>>) {
    let ctx = h.ctx.clone();
    h.state_mut().link.search = search;
    let hold = PathBuf::from("/elsewhere/structure.fig");
    assert!(h.state_mut().loading.begin(&hold));
    let (done, rx) = done();
    assert!(h.state_mut().go_to_link(&ctx, text, Some(done)));
    wait(h, &mut Vec::new(), "the listing to be held", |a| {
        a.link.searches.iter().any(|s| s.found.is_some())
    });
    h.state_mut().link.answer_by = std::time::Duration::ZERO;
    h.step();
    assert!(
        rx.try_recv()
            .unwrap()
            .unwrap_err()
            .contains("still opening")
    );
    h.state_mut().link.answer_by = super::ANSWER_BY;
    (hold, rx)
}

/// Poll without the rest of the UI, so only the search can request another repaint.
fn link_poll_wake(app: &mut App) -> std::time::Duration {
    let ctx = egui::Context::default();
    let mut delay = std::time::Duration::ZERO;
    for _ in 0..5 {
        let mut output = ctx.run_ui(Default::default(), |ui| app.poll_link_searches(ui.ctx()));
        delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        output.textures_delta.clear();
    }
    delay
}

#[test]
fn an_answered_search_waits_for_the_loader_without_polling_its_old_deadline() {
    let mut h = empty_app();
    let (hold, rx) = answered_held_search(&mut h, &figma("structure", "1-4"), listed_structure);
    assert!(link_poll_wake(h.state_mut()) > std::time::Duration::from_millis(250));
    assert_eq!(
        h.state().link.searching(),
        1,
        "the load still has its search"
    );
    // An unanswered held search still needs the deadline wake.
    h.state_mut().link.searches[0].answered = false;
    let wake = link_poll_wake(h.state_mut());
    // egui subtracts its predicted frame time from request_repaint_after's delay.
    assert!(
        wake > std::time::Duration::ZERO && wake <= std::time::Duration::from_millis(250),
        "{wake:?}"
    );
    h.state_mut().loading.finish(&hold);
    assert!(rx.try_recv().is_err());
}

fn listed_sections(_: &[PathBuf], _: &FileHint) -> Vec<(PathBuf, Match)> {
    vec![(fixture("sections.fig"), Match::Name)]
}

#[test]
fn an_answered_search_does_not_keep_an_old_failure_against_a_new_link() {
    let mut h = empty_app();
    let (_, old_rx) = answered_held_search(&mut h, &figma("structure", "1-4"), listed_structure);
    // Another load fails while that search waits forever. It is repaired before a new link
    // asks for it, so a failure recorded for the earlier search must not refuse this one.
    h.state_mut().link_failed(&fixture("sections.fig"), None);
    h.state_mut().link.search = listed_sections;
    let ctx = h.ctx.clone();
    let (done, rx) = done();
    assert!(
        h.state_mut()
            .go_to_link(&ctx, &figma("sections", "1-5"), Some(done))
    );
    wait(&mut h, &mut Vec::new(), "the repaired file to load", |a| {
        a.tabs.iter().any(|t| t.path == fixture("sections.fig"))
    });
    let answer = rx.try_recv().unwrap().unwrap();
    assert!(answer.starts_with("revealed 1:5 in tab"), "{answer}");
    assert!(
        old_rx.try_recv().is_err(),
        "the old caller is not answered twice"
    );
}

fn listed_namesakes(_: &[PathBuf], _: &FileHint) -> Vec<(PathBuf, Match)> {
    vec![
        (fixture("structure.fig"), Match::Name),
        (fixture("sections.fig"), Match::Name),
    ]
}

#[derive(Clone, Copy, Debug)]
enum QuietOutcome {
    FailedCopy,
    Tie,
    MissingNode,
}

fn check_answered_search_stays_quiet(outcome: QuietOutcome) {
    let mut h = match outcome {
        QuietOutcome::MissingNode => harness(),
        _ => empty_app(),
    };
    let node = if matches!(outcome, QuietOutcome::MissingNode) {
        "999-1"
    } else {
        "1-4"
    };
    let search = if matches!(outcome, QuietOutcome::Tie) {
        listed_namesakes
    } else {
        listed_structure
    };
    let before = h.state().tabs.len();
    let (hold, rx) = answered_held_search(&mut h, &figma("structure", node), search);
    if matches!(outcome, QuietOutcome::FailedCopy) {
        h.state_mut().link_failed(&fixture("structure.fig"), None);
    }
    h.state_mut().loading.finish(&hold);
    h.run_steps(3);
    assert_eq!(h.state().link.searching(), 0);
    assert_eq!(h.state().loading.iter().count(), 0, "{outcome:?}: no retry");
    assert_eq!(h.state().tabs.len(), before, "{outcome:?}: no new file");
    assert!(h.state().toasts.is_empty(), "{outcome:?}: no toast");
    assert!(!h.state().link.window, "{outcome:?}: no window");
    assert!(
        h.state().link.choices.is_empty(),
        "{outcome:?}: no file chooser"
    );
    assert!(rx.try_recv().is_err(), "{outcome:?}: no second answer");
}

#[test]
fn an_answered_search_with_a_failed_copy_stays_quiet() {
    check_answered_search_stays_quiet(QuietOutcome::FailedCopy);
}

#[test]
fn an_answered_search_with_a_tie_stays_quiet() {
    check_answered_search_stays_quiet(QuietOutcome::Tie);
}

#[test]
fn an_answered_search_with_a_missing_node_stays_quiet() {
    check_answered_search_stays_quiet(QuietOutcome::MissingNode);
}

/// Paste into the focused command bar, without submitting or stepping the frame yet.
fn paste_command_link(h: &mut Harness<'_, App>, text: &str) {
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::L);
    h.run_steps(3);
    assert!(h.state().command.is_open());
    h.event(egui::Event::Paste(text.to_owned()));
}

#[test]
fn the_command_bar_opens_pasted_links_on_enter_or_click() {
    use egui_kittest::kittest::Queryable;
    for (text, enter) in [
        (figma("structure", "1-25"), true),
        ("figstudio://structure?node-id=1-25".into(), false),
    ] {
        let mut h = harness();
        h.state_mut().link.dirs.clear();
        paste_command_link(&mut h, &format!("  {text}  "));
        h.run_steps(3);
        assert!(
            h.state().tabs[0].selection.is_none(),
            "paste does not navigate"
        );
        assert!(!h.state().link.window);
        assert!(
            h.query_by_label("Open Figma link").is_some(),
            "a link result"
        );
        if enter {
            h.key_press(egui::Key::Enter);
        } else {
            h.get_by_label("Open Figma link").click();
        }
        h.run_steps(3);
        searched(&mut h);
        assert!(!h.state().command.is_open());
        assert_eq!(h.state().tabs[0].selection, Some(Guid(1, 25)));
        assert!(!h.state().link.window, "the link was shown");
        assert!(h.state().link.text.is_empty());
    }
}

#[test]
fn a_link_pasted_with_enter_in_one_frame_uses_the_new_query() {
    let mut h = harness();
    h.state_mut().link.dirs.clear();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::L);
    h.run_steps(3);
    // Harness::event runs a separate frame per event. RawInput puts both in one frame.
    h.input_mut().events.extend([
        egui::Event::Paste(figma("structure", "1-25")),
        egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    h.run_steps(3);
    searched(&mut h);
    assert_eq!(h.state().tabs[0].selection, Some(Guid(1, 25)));
    assert!(!h.state().command.is_open());
}

/// Put the edit and the row's accessibility click in one frame, without moving text focus.
fn edit_command_and_click(h: &mut Harness<'_, App>, text: &str) {
    use egui_kittest::kittest::{NodeT, Queryable};
    let (target_node, target_tree) = h.get_by_label("Open Figma link").accesskit_node().locate();
    h.input_mut().events.extend([
        egui::Event::Paste(text.to_owned()),
        egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            target_node,
            target_tree,
            action: egui::accesskit::Action::Click,
            data: None,
        }),
    ]);
    h.run_steps(3);
}

#[test]
fn a_same_frame_command_edit_keeps_a_click_on_the_unchanged_link() {
    let mut h = harness();
    h.state_mut().link.dirs.clear();
    paste_command_link(&mut h, &figma("structure", "1-25"));
    h.run_steps(3);
    edit_command_and_click(&mut h, " ");
    searched(&mut h);
    assert!(!h.state().command.is_open());
    assert_eq!(h.state().tabs[0].selection, Some(Guid(1, 25)));
    assert!(!h.state().link.window);
}

#[test]
fn a_same_frame_command_edit_discards_a_click_on_a_different_link() {
    let mut h = harness();
    h.state_mut().link.dirs.clear();
    paste_command_link(&mut h, &figma("structure", "1-25"));
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(3);
    edit_command_and_click(&mut h, &figma("structure", "1-4"));
    assert!(h.state().command.is_open());
    assert_eq!(h.state().tabs[0].selection, None);
    assert!(!h.state().link.window);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    searched(&mut h);
    assert_eq!(h.state().tabs[0].selection, Some(Guid(1, 4)));
}

#[test]
fn fileless_figma_urls_do_not_submit_against_the_active_document() {
    use egui_kittest::kittest::Queryable;
    for text in [
        "https://figma.com",
        "https://www.figma.com/?node-id=1-25",
        "https://figma.com/proto/KEY/structure?node-id=1-25",
    ] {
        let mut h = harness();
        h.state_mut().link.dirs.clear();
        h.state_mut().tabs[0].selection = Some(Guid(1, 4));
        paste_command_link(&mut h, text);
        h.run_steps(3);
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert!(h.state().command.is_open(), "not submitted: {text}");
        assert_eq!(h.state().tabs[0].selection, Some(Guid(1, 4)), "{text}");
        assert_eq!(h.state().tabs.len(), 1);
        assert!(!h.state().link.window);
        assert_eq!(h.state().link.searching(), 0);
        assert!(
            h.query_by(|n| {
                n.role() == egui::accesskit::Role::TextInput && n.value().as_deref() == Some(text)
            })
            .is_some(),
            "keep the query for editing: {text}"
        );
    }
}

#[test]
fn the_command_bar_keeps_link_choices_and_errors_in_the_existing_link_window() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["My Design.fig", "My Design (1).fig"] {
        std::fs::copy(fixture("sections.fig"), dir.path().join(name)).unwrap();
    }
    let mut h = harness();
    h.state_mut().link.dirs = vec![dir.path().to_owned()];
    paste_command_link(&mut h, SLUG_LINK);
    h.run_steps(3);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    searched(&mut h);
    assert!(!h.state().command.is_open());
    assert!(h.state().link.window);
    assert_eq!(h.state().link.text, SLUG_LINK);
    assert_eq!(
        h.state().link.choices.len(),
        2,
        "no guess between namesakes"
    );
    assert_eq!(h.state().tabs.len(), 1);

    // Another link replaces this one; failure keeps its text for editing or choosing a folder.
    let missing = figma("No-Such-File", "1-5");
    paste_command_link(&mut h, &missing);
    h.run_steps(3);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    searched(&mut h);
    assert!(h.state().link.window);
    assert_eq!(h.state().link.text, missing);
    assert!(h.state().link.choices.is_empty());
    assert!(h.state().toasts.last().is_some_and(|t| t.error));
    assert_eq!(h.state().tabs.len(), 1);
}

#[test]
fn the_command_bar_opens_a_local_copy_and_shows_the_linked_node() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(fixture("sections.fig"), dir.path().join("My Design.fig")).unwrap();
    let mut h = empty_app();
    h.state_mut().link.dirs = vec![dir.path().to_owned()];
    paste_command_link(&mut h, SLUG_LINK);
    h.run_steps(3);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    wait(&mut h, &mut Vec::new(), "the linked copy to load", |a| {
        a.tabs.len() == 1 && a.tabs[0].selection == Some(Guid(1, 5))
    });
    assert_eq!(h.state().active, 0);
    assert!(!h.state().command.is_open());
    assert!(!h.state().link.window);
}

#[test]
fn command_bar_links_use_the_shared_parser_and_do_not_replace_searches() {
    let h = harness();
    for text in [SLUG_LINK, "figstudio://structure?node-id=1-25"] {
        let rows = crate::command::entries(h.state(), text);
        assert_eq!(rows.len(), 1, "one link action");
        assert_eq!(rows[0].title, "Open Figma link");
    }
    for text in [
        "https://figma.com",
        "https://www.figma.com/design",
        "https://www.figma.com/?node-id=1-25",
        "https://figma.com/community/file/KEY",
        "https://figma.com/embed?url=https://figma.com/design/KEY/structure",
        "https://figma.com/proto/KEY/structure?node-id=1-25",
        "https://figma.com/slides/KEY/structure?node-id=1-25",
        "https://figma.com.example/design/KEY/structure?node-id=1-25",
        "https://example.com/design/KEY/structure?node-id=1-25",
        "javascript:alert(1)",
        "file:///tmp/a.fig",
        "https://figma.com/design/KEY/structure?node-id=bad",
        "https://figma.com/design/KEY/structure?node-id=1-25&node-id=1-4",
        "1:25",
    ] {
        assert!(
            crate::command::entries(h.state(), text)
                .iter()
                .all(|e| e.title != "Open Figma link"),
            "not a supported URL: {text}"
        );
    }
    assert!(
        crate::command::entries(h.state(), "zoom to fit")
            .iter()
            .any(|e| e.title == "Zoom to fit")
    );
}

#[test]
fn open_link_shortcut_uses_the_keymap_without_taking_the_command_bar_keys() {
    use egui::{Key, KeyboardShortcut, Modifiers};
    let mut h = harness();
    let shortcut = h
        .state()
        .prefs
        .keymap
        .for_action(crate::menu::Action::OpenLink);
    assert_eq!(
        shortcut,
        Some(KeyboardShortcut::new(
            Modifiers::COMMAND | Modifiers::SHIFT,
            Key::L
        ))
    );
    assert_eq!(
        h.state()
            .prefs
            .keymap
            .for_action(crate::menu::Action::CommandBar),
        Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::L))
    );
    h.key_press_modifiers(Modifiers::COMMAND, Key::L);
    h.run_steps(3);
    assert!(h.state().command.is_open());
    assert!(!h.state().link.window);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(!h.state().command.is_open());
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::L);
    h.run_steps(3);
    assert!(h.state().link.window);
    assert!(!h.state().command.is_open());
}

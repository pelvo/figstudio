use super::*;
use figstudio_fig::{Document, Guid};
use std::{path::PathBuf, sync::Arc, time::Duration};

fn tabs() -> Vec<Tab> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let doc = Arc::new(Document::open(&path).unwrap());
    vec![
        Tab::new(7, path.clone(), doc.clone(), None, Duration::ZERO),
        Tab::new(9, path, doc, None, Duration::ZERO),
    ]
}

#[test]
fn state_lists_only_received_tabs_with_wire_ids() {
    let mut tabs = tabs();
    tabs[0].select(Some(Guid(1, 25)));
    tabs[0].canvas_rect = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(800., 600.),
    ));
    let ready = HashSet::from([7]);
    let ClientMessage::State { tabs: out, active } = state_message(&tabs, 0, &ready) else {
        panic!("not a state frame");
    };
    assert_eq!(out.len(), 1);
    assert_eq!((out[0].tab, out[0].selection.as_deref()), (7, Some("1:25")));
    assert!(out[0].page.is_some() && out[0].viewport.is_some());
    assert_eq!(active, Some(7));
    // The active tab is not uploaded yet: no active id the bridge would reject.
    let ClientMessage::State { active, .. } = state_message(&tabs, 1, &ready) else {
        panic!()
    };
    assert_eq!(active, None);
}

#[test]
fn fragment_token_is_read_without_leaking_other_keys() {
    assert_eq!(token_from_fragment("#mcp=abc123"), Some("abc123".into()));
    assert_eq!(token_from_fragment("#x=1&mcp=a%2Bb"), Some("a+b".into()));
    for none in ["", "#", "#mcp=", "#token=abc", "#xmcp=abc"] {
        assert_eq!(token_from_fragment(none), None, "{none}");
    }
}

#[test]
fn chunks_cover_the_file_exactly() {
    assert_eq!(chunk_ranges(0).count(), 0);
    assert_eq!(chunk_ranges(1).collect::<Vec<_>>(), [(0, 1)]);
    assert_eq!(chunk_ranges(CHUNK).collect::<Vec<_>>(), [(0, CHUNK)]);
    assert_eq!(
        chunk_ranges(CHUNK + 5).collect::<Vec<_>>(),
        [(0, CHUNK), (CHUNK, 5)]
    );
    let big = 10 * CHUNK + 3;
    assert_eq!(chunk_ranges(big).map(|r| r.1).sum::<u64>(), big);
}

#[test]
fn reveal_rejects_bad_ids_and_closed_tabs_without_touching_state() {
    let mut tabs = tabs();
    let mut active = 0;
    for bad in ["", "abc", "1:2:3", "1-2", "https://x/y", "1:25/2:3"] {
        assert!(
            reveal_wire(&mut tabs, &mut active, 9, bad).is_err(),
            "{bad}"
        );
    }
    assert_eq!((active, tabs[1].selection), (0, None));
    assert_eq!(
        reveal_wire(&mut tabs, &mut active, 42, "1:25"),
        Err("tab closed".into())
    );
    reveal_wire(&mut tabs, &mut active, 9, "1:25").unwrap();
    assert_eq!((active, tabs[1].selection), (1, Some(Guid(1, 25))));
}

#[test]
fn close_codes_name_the_cause() {
    assert!(close_reason(4401).contains("token"));
    assert!(close_reason(4426).contains("version"));
    assert!(close_reason(1006).contains("disconnected"));
}

#[test]
fn a_failed_connect_names_local_network_access_and_the_https_limit() {
    let message = connect_failed_message();
    assert!(message.contains("local network access"), "{message}");
    assert!(message.contains("https -> ws://"), "{message}");
    assert!(message.contains("figstudio-mcp --bridge"), "{message}");
}

#[test]
fn upload_names_never_carry_separators_or_overlong_text() {
    assert_eq!(upload_name("a.fig"), "a.fig");
    assert_eq!(upload_name("a/b\\c:d.fig"), "a_b_c_d.fig");
    assert_eq!(upload_name("x\ny"), "x_y");
    for bad in ["", ".", ".."] {
        assert_eq!(upload_name(bad), "untitled");
    }
    let long = "あ".repeat(300);
    let name = upload_name(&long);
    assert!(name.len() <= 200 && name.chars().all(|c| c == 'あ'));
}

#[test]
fn two_files_with_one_name_stay_bound_to_their_own_tabs() {
    let mut uploads = Uploads::new();
    let name = PathBuf::from("same.fig");
    let first = uploads.pick("first");
    uploads.start(name.clone(), first);
    uploads.bind(&name, 1);
    // A second file of the same name while tab 1 is open must not replace tab 1's file.
    let second = uploads.pick("second");
    uploads.start(name.clone(), second);
    uploads.bind(&name, 2);
    assert_eq!(uploads.get(1), Some("first"));
    assert_eq!(uploads.get(2), Some("second"));
    uploads.forget(1);
    assert_eq!(uploads.get(1), None);
    assert_eq!(uploads.get(2), Some("second"));
}

#[test]
fn a_second_pick_of_a_loading_name_does_not_take_the_first_load() {
    let mut uploads = Uploads::new();
    let name = PathBuf::from("same.fig");
    let a = uploads.pick("A");
    let b = uploads.pick("B");
    // `Pending` lets A's load start and ignores B's.
    uploads.start(name.clone(), a);
    uploads.discard(b);
    uploads.bind(&name, 1);
    assert_eq!(uploads.get(1), Some("A"));
    // B's start after the discard finds nothing, so it can never bind.
    uploads.start(name.clone(), b);
    uploads.bind(&name, 2);
    assert_eq!(uploads.get(2), None);
}

#[test]
fn a_failed_load_releases_its_file() {
    let mut uploads = Uploads::new();
    let name = PathBuf::from("bad.fig");
    let a = uploads.pick("A");
    uploads.start(name.clone(), a);
    uploads.fail(&name);
    uploads.bind(&name, 1);
    assert_eq!(uploads.get(1), None);
}

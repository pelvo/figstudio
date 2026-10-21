#[path = "../tests/support/mod.rs"]
mod support;
use super::*;
use figstudio_fig::{Node, Val};
use support::{document, filled_rectangle_at, node};

fn screen(id: u32, parent: u32, x: f64) -> Node {
    let mut n = filled_rectangle_at(id, parent, (1.0, 0.0, 0.0), x, 0.0, 100.0, 100.0);
    n.data.set("type", Val::Str("FRAME".into()));
    n.data.set("name", Val::Str("Screen".into()));
    Node::from_data(n.data).unwrap()
}

#[test]
fn discovery_matches_reparented_and_recreated_screens_not_nested_frames_or_text() {
    let old = document(vec![
        node(2, Some(1), "SECTION", ""),
        screen(3, 2, 0.0),
        screen(4, 3, 0.0),
        node(5, Some(1), "TEXT", ""),
    ]);
    let new = document(vec![
        node(6, Some(1), "SECTION", ""),
        screen(3, 6, 500.0),
        screen(4, 3, 0.0),
    ]);
    assert_eq!(
        match_screens(&old, &new, Guid(0, 1)),
        vec![ScreenPair {
            old: Some(Guid(0, 3)),
            new: Some(Guid(0, 3)),
            matched_by: Some(MatchBy::Id)
        }]
    );
    let new = document(vec![screen(7, 1, 200.0)]);
    assert_eq!(
        match_screens(&old, &new, Guid(0, 1)),
        vec![ScreenPair {
            old: Some(Guid(0, 3)),
            new: Some(Guid(0, 7)),
            matched_by: Some(MatchBy::NameSize)
        }]
    );
}

#[test]
fn ambiguous_names_pair_by_nearest_distance_then_guid_and_keep_unmatched() {
    let old = document(vec![
        screen(2, 1, 0.0),
        screen(3, 1, 300.0),
        node(9, Some(1), "GROUP", "Removed"),
    ]);
    let new = document(vec![
        screen(7, 1, 290.0),
        screen(8, 1, 10.0),
        node(10, Some(1), "INSTANCE", "Added"),
    ]);
    let pairs = match_screens(&old, &new, Guid(0, 1));
    assert_eq!(
        pairs.iter().map(|p| (p.old, p.new)).collect::<Vec<_>>(),
        vec![
            (Some(Guid(0, 2)), Some(Guid(0, 8))),
            (Some(Guid(0, 3)), Some(Guid(0, 7))),
            (Some(Guid(0, 9)), None),
            (None, Some(Guid(0, 10)))
        ]
    );
    let old = document(vec![screen(2, 1, 0.0), screen(3, 1, 0.0)]);
    let new = document(vec![screen(7, 1, 0.0), screen(8, 1, 0.0)]);
    assert_eq!(
        match_screens(&old, &new, Guid(0, 1))[0].new,
        Some(Guid(0, 7))
    );
}

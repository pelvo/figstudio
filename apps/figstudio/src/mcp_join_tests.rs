//! Native tests for the linked-room join decisions (`mcp_join.rs`).
use super::*;

fn room(hex: char) -> String {
    format!("rm_{}{}", hex.to_string().repeat(63), hex)
}

#[test]
fn a_link_parses_token_room_and_the_strip_together() {
    let a = room('a');
    let b = room('b');
    // Both keys together: both are read, and the room key is still consumed.
    assert_eq!(
        parse_link(Some(&format!("#mcp=pair&room={a}"))),
        Link {
            token: Some("pair".to_owned()),
            room: Some(a.clone()),
            stripped: Some("#mcp=pair".to_owned()),
        }
    );
    assert_eq!(
        parse_link(Some(&format!("#room={a}"))),
        Link {
            token: None,
            room: Some(a),
            stripped: Some(String::new()),
        }
    );
    // Invalid room values never propose; the stray key is still consumed.
    assert_eq!(
        parse_link(Some("#room=rm_short")),
        Link {
            token: None,
            room: None,
            stripped: Some(String::new()),
        }
    );
    // A room link with other keys keeps them; a full URL works like a bare fragment.
    assert_eq!(
        parse_link(Some(&format!("https://example.com/?x=1#z=2&room={b}"))),
        Link {
            token: None,
            room: Some(b),
            stripped: Some("#z=2".to_owned()),
        }
    );
    assert_eq!(parse_link(None), Link::default());
    assert_eq!(parse_link(Some("")), Link::default());
    assert_eq!(parse_link(Some("#mcp=pair")).stripped, None);
}

#[test]
fn stripping_removes_only_the_room_key() {
    let a = room('a');
    for (fragment, stripped) in [
        (format!("#room={a}"), Some(String::new())),
        ("#room=".to_owned(), Some(String::new())),
        ("#room=rm_short".to_owned(), Some(String::new())),
        (format!("#a=1&room={a}"), Some("#a=1".to_owned())),
        (format!("#room={a}&a=1"), Some("#a=1".to_owned())),
        ("#mcp=t".to_owned(), None),
        ("#mcp=t&x=1".to_owned(), None),
        (String::new(), None),
    ] {
        assert_eq!(strip_room_key(&fragment), stripped, "{fragment}");
    }
}

#[test]
fn a_pairing_being_applied_wins_but_a_stale_one_does_not() {
    let a = room('a');
    let b = room('b');
    // A new room, no pairing: propose it.
    assert_eq!(proposed_room(Some(a.clone()), false, &b), Some(a.clone()));
    // A pairing being applied (a fresh `#mcp=` token) wins over the link.
    assert_eq!(proposed_room(Some(a.clone()), true, &b), None);
    // An unchanged `#mcp=` key is stale: the link still proposes (finding 2).
    assert_eq!(proposed_room(Some(a.clone()), false, &a), None);
    assert_eq!(proposed_room(None, false, &b), None);
    assert_eq!(proposed_room(Some(a.clone()), true, &a), None);
}

#[test]
fn the_dialog_text_never_holds_the_whole_capability() {
    let room = room('f');
    let proposal = RoomProposal::new(room.clone());
    assert_eq!(proposal.room(), room);
    assert_eq!(proposal.clone().into_room(), room);
    let masked = proposal.masked();
    assert!(
        masked.starts_with("rm_") && masked.contains('…'),
        "{masked}"
    );
    assert_ne!(masked, room);
    assert!(!masked.contains(&room[3..40]), "{masked}");
}

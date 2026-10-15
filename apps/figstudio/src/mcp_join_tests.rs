//! Native tests for the linked-room join decisions (`mcp_join.rs`).
use super::*;

const ZEROS: &str = "ex_0000000000000000000000000000000000000000000000000000000000000000";
const SEQUENCE: &str = "ex_000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn pair(executor: &str) -> ExecutorPair {
    ExecutorPair::from_executor(executor).unwrap()
}

/// The two fragment keys, without the `#` so they can follow other keys.
fn keys(pair: &ExecutorPair) -> String {
    format!("room={}&exec={}", pair.room(), pair.executor())
}

fn link_of(pair: &ExecutorPair) -> String {
    format!("#{}", keys(pair))
}

#[test]
fn a_link_parses_token_pair_and_the_strip_together() {
    let a = pair(ZEROS);
    let b = pair(SEQUENCE);
    // Both keys together: both are read, and the secret keys are still consumed.
    assert_eq!(
        parse_link(Some(&format!("#mcp=pair&{}", keys(&a)))),
        Link {
            token: Some("pair".to_owned()),
            pair: Some(a.clone()),
            stripped: Some("#mcp=pair".to_owned()),
        }
    );
    assert_eq!(
        parse_link(Some(&link_of(&a))),
        Link {
            token: None,
            pair: Some(a),
            stripped: Some(String::new()),
        }
    );
    // Invalid or mixed values never propose; the stray keys are still consumed.
    for fragment in [
        "#room=rm_short&exec=ex_also_short",
        &format!("#room={}", b.room()),
        &format!("#exec={SEQUENCE}"),
        &format!("#room=rm_{}&exec={SEQUENCE}", "0".repeat(64)),
    ] {
        assert_eq!(
            parse_link(Some(fragment)),
            Link {
                token: None,
                pair: None,
                stripped: Some(String::new()),
            },
            "{fragment}"
        );
    }
    // A pair link with other keys keeps them; a full URL works like a bare fragment.
    let with_keys = format!("https://example.com/?x=1#z=2&{}", keys(&b));
    assert_eq!(
        parse_link(Some(&with_keys)),
        Link {
            token: None,
            pair: Some(b),
            stripped: Some("#z=2".to_owned()),
        }
    );
    assert_eq!(parse_link(None), Link::default());
    assert_eq!(parse_link(Some("")), Link::default());
    assert_eq!(parse_link(Some("#mcp=pair")).stripped, None);
}

#[test]
fn stripping_removes_both_secret_keys_even_when_they_are_invalid() {
    let a = pair(ZEROS);
    for (fragment, stripped) in [
        (link_of(&a), Some(String::new())),
        ("#room=".to_owned(), Some(String::new())),
        ("#exec=".to_owned(), Some(String::new())),
        (
            "#room=rm_short&exec=ex_short".to_owned(),
            Some(String::new()),
        ),
        // A mixed link with one valid half still loses both halves.
        (
            format!("#room={}&exec=ex_short", a.room()),
            Some(String::new()),
        ),
        (format!("#a=1&{}", keys(&a)), Some("#a=1".to_owned())),
        (format!("{}&a=1", keys(&a)), Some("#a=1".to_owned())),
        ("#mcp=t".to_owned(), None),
        ("#mcp=t&x=1".to_owned(), None),
        (String::new(), None),
    ] {
        assert_eq!(strip_secret_keys(&fragment), stripped, "{fragment}");
    }
}

#[test]
fn a_pairing_being_applied_wins_but_a_stale_one_does_not() {
    let a = pair(ZEROS);
    let b = pair(SEQUENCE);
    // A new pair, no pairing: propose it.
    assert_eq!(
        proposed_pair(Some(a.clone()), false, Some(&b)),
        Some(a.clone())
    );
    // A pairing being applied (a fresh `#mcp=` token) wins over the link.
    assert_eq!(proposed_pair(Some(a.clone()), true, Some(&b)), None);
    // An unchanged `#mcp=` key is stale: the link still proposes (finding 2).
    assert_eq!(proposed_pair(Some(a.clone()), false, Some(&a)), None);
    assert_eq!(proposed_pair(None, false, Some(&b)), None);
    assert_eq!(proposed_pair(Some(a.clone()), true, Some(&a)), None);
    // No pair held yet (a failed load): any valid link still proposes.
    assert_eq!(proposed_pair(Some(a), false, None), Some(pair(ZEROS)));
}

#[test]
fn the_dialog_text_never_holds_the_whole_capability() {
    let pair = pair(SEQUENCE);
    let proposal = RoomProposal::new(pair.clone());
    assert_eq!(proposal.pair().room(), pair.room());
    assert_eq!(proposal.clone().into_pair(), pair);
    assert_eq!(proposal.pair(), &pair);
    let masked = proposal.masked();
    assert!(
        masked.starts_with("rm_") && masked.contains('…'),
        "{masked}"
    );
    let room = pair.room();
    assert_ne!(masked, room);
    assert!(!masked.contains(&room[3..40]), "{masked}");
    // The proposal's Debug (and the pair's) keeps the executor secret out.
    let debug = format!("{proposal:?}");
    assert!(
        !debug.contains(SEQUENCE) && !debug.contains(&SEQUENCE[3..]),
        "{debug}"
    );
}

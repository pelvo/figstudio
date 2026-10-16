//! Native tests for the executor credential (`mcp_pair.rs`). The derivation vectors are
//! the shared contract with `web/worker/executor-auth.test.mjs`: same domain, same bytes,
//! same room, on both sides.
use super::*;

/// `ex_` + 64 zeros — the fixed vector both Rust and JS must reproduce.
const ZEROS: &str = "ex_0000000000000000000000000000000000000000000000000000000000000000";
const SEQUENCE: &str = "ex_000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

#[test]
fn the_derivation_matches_the_shared_vector() {
    assert_eq!(
        derive_room(&ZEROS[3..]).as_deref(),
        Some("rm_f387ed88cf9a2c064c95e23d5b96cb76e2c9af734b41febc643dd57b90fb7525")
    );
    assert_eq!(
        derive_room(&SEQUENCE[3..]).as_deref(),
        Some("rm_9851486d85fc5fe5c0d9082ab07c4d165e75120278631e4e221905f821b180c1")
    );
    // Only lower-case hex of exactly 32 bytes derives; nothing else does. (Upper-casing
    // the all-zeros hex would be a no-op, so the case check uses a hex string with letters.)
    for bad in [
        "",
        "abc",
        &SEQUENCE[3..].to_uppercase(),
        &ZEROS[3..][..63],
        "0x00",
    ] {
        assert_eq!(derive_room(bad), None, "{bad}");
    }
}

#[test]
fn a_generated_pair_has_both_formats_and_derives_back() {
    let pair = ExecutorPair::generate().unwrap();
    assert!(valid_executor(pair.executor()), "{}", pair.executor());
    assert_ne!(pair.executor(), ZEROS, "entropy produced the zero vector");
    let room = pair.room();
    assert!(crate::mcp_wire::valid_room(&room));
    // The derivation is a function of the executor alone.
    let again = ExecutorPair::from_executor(pair.executor()).unwrap();
    assert_eq!(again.room(), room);
    assert_eq!(again, pair);
    // The raw hex offered as the second subprotocol is the executor without the prefix.
    assert_eq!(pair.executor_hex().len(), 64);
    assert_eq!(pair.executor(), format!("ex_{}", pair.executor_hex()));
}

#[test]
fn only_a_well_formed_executor_token_is_accepted() {
    for good in [ZEROS, SEQUENCE] {
        assert!(ExecutorPair::from_executor(good).is_some());
    }
    for bad in [
        "rm_0000000000000000000000000000000000000000000000000000000000000000",
        &ZEROS.to_uppercase(),
        &ZEROS[..66],
        &format!("{ZEROS}0"),
        "ex_",
        "",
    ] {
        assert!(ExecutorPair::from_executor(bad).is_none(), "{bad}");
    }
}

#[test]
fn debug_and_the_subprotocols_never_expose_the_secret() {
    let pair = ExecutorPair::from_executor(SEQUENCE).unwrap();
    let debug = format!("{pair:?}");
    assert!(debug.contains("<redacted>"), "{debug}");
    assert!(!debug.contains(SEQUENCE), "{debug}");
    assert!(!debug.contains(&SEQUENCE[3..]), "{debug}");
    // The marker is constant and first; the executor hex is offered, never logged.
    assert_eq!(
        pair.subprotocols(),
        [BRIDGE_PROTOCOL.to_owned(), SEQUENCE[3..].to_owned()]
    );
}

#[test]
fn a_fragment_needs_both_keys_and_they_must_match() {
    let pair = ExecutorPair::from_executor(SEQUENCE).unwrap();
    let room = pair.room();
    let other = ExecutorPair::from_executor(ZEROS).unwrap().room();
    let upper = SEQUENCE.to_uppercase();
    // Both keys, matching: a proposal.
    assert_eq!(
        pair_from_fragment(&format!("#room={room}&exec={SEQUENCE}")),
        Some(pair.clone())
    );
    // Order, other keys and a full URL do not matter.
    assert_eq!(
        pair_from_fragment(&format!(
            "https://example.com/?x=1#exec={SEQUENCE}&mcp=t&room={room}"
        )),
        Some(pair.clone())
    );
    // Mixed or partial credentials never propose: wrong room, missing key, invalid values.
    for bad in [
        format!("#room={other}&exec={SEQUENCE}"),
        format!("#room={room}"),
        format!("#exec={SEQUENCE}"),
        format!("#room=rm_short&exec={SEQUENCE}"),
        format!("#room={room}&exec=ex_short"),
        format!("#room={room}&exec={upper}"),
    ] {
        assert_eq!(pair_from_fragment(&bad), None, "{bad}");
    }
}

#[test]
fn typed_input_accepts_the_executor_or_a_matching_link_only() {
    let pair = ExecutorPair::from_executor(SEQUENCE).unwrap();
    let room = pair.room();
    assert_eq!(pair_from_input(SEQUENCE), Some(pair.clone()));
    assert_eq!(
        pair_from_input(&format!("  {SEQUENCE}  ")),
        Some(pair.clone())
    );
    assert_eq!(
        pair_from_input(&format!("https://example.com/#room={room}&exec={SEQUENCE}")),
        Some(pair)
    );
    // The caller capability alone — the MCP URL or a bare room — is not the executor.
    for rejected in [
        format!("https://example.com/mcp/{room}"),
        room.clone(),
        format!("https://example.com/#room={room}"),
        format!("https://example.com/#room={room}&exec={ZEROS}"),
        format!("wss://example.com/bridge/{room}"),
        String::new(),
    ] {
        assert_eq!(pair_from_input(&rejected), None, "{rejected}");
    }
}

#[test]
fn the_join_link_round_trips_and_checks_its_own_keys() {
    let pair = ExecutorPair::from_executor(SEQUENCE).unwrap();
    let link = pair_link("https:", "figstudio.example", &pair).unwrap();
    assert_eq!(
        link,
        format!(
            "https://figstudio.example#room={}&exec={SEQUENCE}",
            pair.room()
        )
    );
    assert_eq!(pair_from_fragment(&link), Some(pair.clone()));
    assert_eq!(pair_from_input(&link), Some(pair));
    assert_eq!(
        pair_link(
            "file:",
            "figstudio.example",
            &ExecutorPair::generate().unwrap()
        ),
        None
    );
    assert_eq!(
        pair_link("https:", "", &ExecutorPair::generate().unwrap()),
        None
    );
}

#[test]
fn loading_decides_between_use_upgrade_and_visible_failure() {
    let pair = ExecutorPair::from_executor(SEQUENCE).unwrap();
    let legacy =
        Some("rm_0000000000000000000000000000000000000000000000000000000000000000".to_owned());
    // Fresh install: nothing stored.
    assert!(matches!(plan(None, None), Load::Fresh));
    // Legacy room present: always upgrade, whatever the legacy value holds.
    assert!(matches!(plan(None, legacy.clone()), Load::Upgrade));
    assert!(matches!(
        plan(None, Some("malformed".to_owned())),
        Load::Upgrade
    ));
    // Valid v2 wins, and asks the legacy key to be cleaned up when it lingers.
    match plan(Some(SEQUENCE.to_owned()), None) {
        Load::Use {
            pair: found,
            legacy,
        } => {
            assert_eq!(found, pair);
            assert!(!legacy);
        }
        _ => panic!("a valid executor must be used"),
    }
    match plan(Some(SEQUENCE.to_owned()), legacy.clone()) {
        Load::Use { legacy, .. } => assert!(legacy),
        _ => panic!("a valid executor must be used"),
    }
    // A corrupt v2 value fails visibly; a valid legacy room is not a fallback.
    assert!(matches!(
        plan(Some("malformed".to_owned()), legacy),
        Load::Corrupt
    ));
    assert!(matches!(
        plan(Some(ZEROS[..66].to_owned()), None),
        Load::Corrupt
    ));
}

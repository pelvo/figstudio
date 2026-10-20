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
fn relay_keys_are_unique_without_exposing_paths() {
    let mut tabs = tabs();
    tabs[0].path = PathBuf::from("folder-a/design.fig");
    tabs[1].path = PathBuf::from("folder-b/design.fig");
    assert_eq!(relay_keys(&tabs), ["design~7", "design~9"]);
    tabs.reverse();
    assert_eq!(relay_keys(&tabs), ["design~9", "design~7"]);
    tabs[0].path = PathBuf::from("other.fig");
    assert_eq!(relay_keys(&tabs), ["other", "design"]);
}

#[test]
fn room_validation_and_urls() {
    let room = format!("rm_{}", "a".repeat(64));
    assert!(valid_room(&room));
    for bad in [
        "",
        "rm_",
        &format!("rm_{}", "a".repeat(63)),
        &format!("rm_{}", "a".repeat(65)),
        &format!("rm_{}", "G".repeat(64)),
        &format!("xx_{}", "a".repeat(64)),
        &format!("rm_{}", "é".repeat(32)),
    ] {
        assert!(!valid_room(bad), "{bad}");
    }
    for (protocol, ws) in [("https:", "wss"), ("http:", "ws")] {
        assert_eq!(
            relay_urls(protocol, "localhost:8787", &room),
            Some((
                format!("{ws}://localhost:8787/bridge/{room}"),
                format!("{protocol}//localhost:8787/mcp/{room}")
            ))
        );
    }
    assert_eq!(relay_urls("file:", "localhost", &room), None);
    assert_eq!(relay_urls("https:", "", &room), None);
    assert_eq!(relay_urls("https:", "localhost", "bad"), None);
}

#[test]
fn only_the_workers_probe_answer_makes_cloud_relay_available() {
    // The relay Worker's answer (web/worker.mjs: 204 + `figstudio-relay: 1`).
    for status in [200, 204] {
        assert_eq!(
            CloudRelay::from_probe(Some(status), Some("1")),
            CloudRelay::Available
        );
    }
    // Static-only hosts answer for good: a missing asset, an SPA fallback page, an empty
    // 204 from some other server, a header value the Worker never sends.
    for (status, header) in [
        (404, None),
        (200, None),
        (204, None),
        (204, Some("")),
        (204, Some("  ")),
        (204, Some("0")),
        (204, Some("true")),
        (302, Some("1")),
        (404, Some("1")),
    ] {
        assert_eq!(
            CloudRelay::from_probe(Some(status), header),
            CloudRelay::Absent,
            "{status} {header:?}"
        );
    }
    // A failed request (offline, timed out, a sign-in redirect the page may not follow)
    // or a server error may pass: the next connect asks again.
    for (status, header) in [
        (None, None),
        (None, Some("1")),
        (Some(500), Some("1")),
        (Some(502), None),
        (Some(503), None),
        (Some(429), None),
    ] {
        assert_eq!(
            CloudRelay::from_probe(status, header),
            CloudRelay::Unreachable,
            "{status:?} {header:?}"
        );
    }
}

#[test]
fn the_probe_path_and_header_match_the_relay_worker() {
    // The page and the Worker must agree, or every deploy looks static-only.
    let worker = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/worker.mjs"),
    )
    .unwrap();
    assert!(
        worker.contains(&format!("RELAY_PROBE_HEADER = \"{RELAY_PROBE_HEADER}\"")),
        "web/worker.mjs must set the {RELAY_PROBE_HEADER} header"
    );
    assert!(
        worker.contains(&format!("[RELAY_PROBE_HEADER]: \"{RELAY_PROBE_VALUE}\"")),
        "web/worker.mjs must send {RELAY_PROBE_HEADER}: {RELAY_PROBE_VALUE}"
    );
    assert!(
        worker.contains(&format!("path === \"{RELAY_PROBE_PATH}\"")),
        "web/worker.mjs must answer the probe at {RELAY_PROBE_PATH}"
    );
}

#[test]
fn cloud_relay_is_offered_until_the_origin_is_known_to_lack_it() {
    assert_eq!(CloudRelay::default(), CloudRelay::Unknown);
    let cases = [
        (CloudRelay::Unknown, true),
        (CloudRelay::Available, true),
        (CloudRelay::Absent, false),
        (CloudRelay::Unreachable, true),
    ];
    for (state, offered) in cases {
        assert_eq!(state.offered(), offered, "{state:?}");
    }
}

#[test]
fn a_cloud_connect_asked_before_the_probe_answers_waits_for_it() {
    // Connect, Join or Apply pressed before `GET /mcp/` answered.
    let mut gate = CloudGate::default();
    assert_eq!(
        gate.step(CloudRelay::Unknown, true, true),
        CloudStep::default()
    );
    assert!(gate.waiting());
    assert_eq!(
        gate.step(CloudRelay::Unknown, true, false),
        CloudStep::default()
    );
    let connect = CloudStep {
        connect: true,
        ..CloudStep::default()
    };
    assert_eq!(gate.step(CloudRelay::Available, true, false), connect);
    assert!(!gate.waiting());
    // Answered once: later frames do not connect again on their own, and a new request
    // goes straight through.
    assert_eq!(
        gate.step(CloudRelay::Available, true, false),
        CloudStep::default()
    );
    assert_eq!(gate.step(CloudRelay::Available, true, true), connect);
}

#[test]
fn the_page_loads_own_connect_waits_and_is_quiet_on_a_static_only_origin() {
    // A deploy with the Worker: the page load's connect opens once the probe answers.
    let mut gate = CloudGate::page_load(true);
    assert!(gate.waiting());
    assert_eq!(
        gate.step(CloudRelay::Unknown, true, false),
        CloudStep::default()
    );
    assert!(gate.step(CloudRelay::Available, true, false).connect);
    // A static-only deploy answers every load: leave Cloud relay without a toast.
    let mut gate = CloudGate::page_load(true);
    assert_eq!(
        gate.step(CloudRelay::Absent, true, false),
        CloudStep {
            leave: true,
            ..CloudStep::default()
        }
    );
    // A failed probe may hide a deploy with the Worker: say so.
    let mut gate = CloudGate::page_load(true);
    assert_eq!(
        gate.step(CloudRelay::Unreachable, true, false).notice,
        Some(CLOUD_RELAY_UNREACHABLE)
    );
    // A page load that did not choose Cloud relay never connects it.
    let mut gate = CloudGate::page_load(false);
    assert!(!gate.waiting());
    assert_eq!(
        gate.step(CloudRelay::Available, true, false),
        CloudStep::default()
    );
}

#[test]
fn a_cloud_connect_on_an_origin_without_the_worker_leaves_cloud_relay() {
    let leave = |notice| CloudStep {
        leave: true,
        notice,
        ..CloudStep::default()
    };
    // Connect pressed while the page load's own connect waits, or a Join of a
    // `#room=&exec=` link after the answer: no socket of either kind, and a toast.
    let mut gate = CloudGate::page_load(true);
    gate.step(CloudRelay::Unknown, true, true);
    assert_eq!(
        gate.step(CloudRelay::Absent, true, false),
        leave(Some(NO_CLOUD_RELAY))
    );
    assert_eq!(
        gate.step(CloudRelay::Absent, true, true),
        leave(Some(NO_CLOUD_RELAY))
    );
    // Nobody asked: leave quietly.
    assert_eq!(gate.step(CloudRelay::Absent, true, false), leave(None));
    // The local bridge and the local proxy still connect on such an origin.
    assert_eq!(
        gate.step(CloudRelay::Absent, false, true),
        CloudStep {
            connect: true,
            ..CloudStep::default()
        }
    );
}

#[test]
fn a_failed_probe_is_asked_again_on_the_next_connect_never_in_a_loop() {
    let mut gate = CloudGate::default();
    gate.step(CloudRelay::Unknown, true, true);
    // The waiting connect gets one toast; nothing probes again on its own.
    assert_eq!(
        gate.step(CloudRelay::Unreachable, true, false),
        CloudStep {
            notice: Some(CLOUD_RELAY_UNREACHABLE),
            ..CloudStep::default()
        }
    );
    assert_eq!(
        gate.step(CloudRelay::Unreachable, true, false),
        CloudStep::default()
    );
    // Connect asks the origin again and waits for that answer.
    assert_eq!(
        gate.step(CloudRelay::Unreachable, true, true),
        CloudStep {
            probe: true,
            ..CloudStep::default()
        }
    );
    assert!(gate.waiting());
    assert!(gate.step(CloudRelay::Available, true, false).connect);
}

#[test]
fn a_mode_switch_in_the_frame_the_probe_answers_still_cancels_the_waiting_connect() {
    // Frame start (before the menu) only applies an Absent answer; it never releases a
    // connect, so Local bridge picked in this frame's menu cannot open the upload bridge.
    let mut gate = CloudGate::page_load(true);
    assert_eq!(
        gate.before_menu(CloudRelay::Available, true),
        CloudStep::default()
    );
    assert!(gate.waiting());
    assert_eq!(
        gate.step(CloudRelay::Available, false, false),
        CloudStep::default()
    );
    // Without the switch, `sync` releases it in the same frame.
    let mut gate = CloudGate::page_load(true);
    gate.before_menu(CloudRelay::Available, true);
    assert!(gate.step(CloudRelay::Available, true, false).connect);
    // An Absent answer leaves Cloud relay before the menu draws.
    let mut gate = CloudGate::page_load(true);
    assert_eq!(
        gate.before_menu(CloudRelay::Absent, true),
        CloudStep {
            leave: true,
            ..CloudStep::default()
        }
    );
}

#[test]
fn leaving_cloud_relay_drops_a_connect_still_waiting_for_the_probe() {
    let mut gate = CloudGate::default();
    gate.step(CloudRelay::Unknown, true, true);
    // Local bridge picked (or a `#mcp=` pairing arrived) before the answer.
    assert_eq!(
        gate.step(CloudRelay::Unknown, false, false),
        CloudStep::default()
    );
    assert!(!gate.waiting());
    assert_eq!(
        gate.step(CloudRelay::Available, true, false),
        CloudStep::default()
    );
}

#[test]
fn relay_rpc_round_trips_json_and_notifications() {
    use serde_json::{Value, json};
    let request = json!({"jsonrpc":"2.0", "id":7, "method":"ping"});
    let frame = json!({"type":"rpc", "id":"request-1", "body":request});
    let (id, body) = parse_rpc(&frame.to_string()).unwrap();
    assert_eq!(body, request);
    let response = json!({"jsonrpc":"2.0", "id":7, "result":{}});
    let reply: Value =
        serde_json::from_str(&rpc_reply(&id, Some(&response.to_string())).unwrap()).unwrap();
    assert_eq!(
        reply,
        json!({"type":"rpc_reply", "id":"request-1", "status":200, "body":response})
    );
    let reply: Value = serde_json::from_str(&rpc_reply(&id, None).unwrap()).unwrap();
    assert_eq!(
        reply,
        json!({"type":"rpc_reply", "id":"request-1", "status":202, "body":null})
    );
    assert!(rpc_reply(&id, Some("bad JSON")).is_err());
    assert_eq!(
        serde_json::from_str::<Value>(relay_ready()).unwrap(),
        json!({"type":"ready","protocol":1})
    );
    let tabs = tabs();
    let v2: Value = serde_json::from_str(&relay_ready_v2(
        "0123456789abcdef0123456789abcdef",
        "Office\n\u{202E} · Chrome",
        "deadbeefcafebabe",
        true,
        &tabs,
    ))
    .unwrap();
    assert_eq!(v2["type"], "ready");
    assert_eq!(v2["protocol"], 2);
    assert_eq!(v2["device"]["id"], "0123456789abcdef0123456789abcdef");
    assert_eq!(v2["device"]["label"], "Office  · Chrome");
    assert_eq!(v2["instance"], "deadbeefcafebabe");
    assert_eq!(v2["focused"], true);
    assert_eq!(v2["docs"].as_array().unwrap().len(), 2);
    assert_eq!(v2["docs"][0]["tab"], 7);
    assert_eq!(v2["docs"][0]["fileKey"], "structure~7");
    assert_eq!(v2["docs"][1]["fileKey"], "structure~9");
    let key0 = v2["docs"][0]["key"].as_str().unwrap();
    let key1 = v2["docs"][1]["key"].as_str().unwrap();
    assert_eq!(key0.len(), 16);
    assert!(
        key0.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "{key0}"
    );
    // Both tabs opened the same `structure.fig` fixture, so their SHA-256 doc keys match.
    assert_eq!(key0, key1);
    assert!(v2["docs"][0]["pages"].as_u64().unwrap() >= 1);
}

#[test]
fn relay_rpc_rejects_malformed_envelopes() {
    for frame in [
        "not JSON",
        "null",
        "[]",
        r#"{"type":"rpc","body":{}}"#,
        r#"{"type":"rpc","id":1,"body":{}}"#,
        r#"{"type":"rpc","id":"","body":{}}"#,
        r#"{"type":"rpc","id":"r","body":null}"#,
        r#"{"type":"rpc","id":"r","body":"{}"}"#,
        r#"{"type":"other","id":"r","body":{}}"#,
    ] {
        assert!(parse_rpc(frame).is_none(), "{frame}");
    }
}

#[test]
fn relay_reveal_applies_selection_fit_and_active_tab() {
    let mut tabs = tabs();
    // A layer: a page ID switches page instead (mcp_state_tests.rs).
    let node = Guid(1, 25);
    let mut active = 0;
    crate::mcp_state::reveal(&mut tabs, &mut active, 9, node).unwrap();
    assert_eq!(active, 1);
    assert_eq!(tabs[1].selection, Some(node));
    assert_eq!(tabs[1].fit_node, Some(node));
    assert_eq!(tabs[0].selection, None);
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
fn oversized_replies_become_an_error_result_for_the_same_call() {
    use serde_json::{Value, json};
    // 4 MiB of CJK is 12 MiB as UTF-8 JSON: over the 7.5 MiB tab-side reply budget.
    let big = json!({"jsonrpc":"2.0", "id":42, "result":{"content":[
        {"type":"text","text":"界".repeat(4 * 1024 * 1024)}]}});
    let frame = rpc_reply("req-1", Some(&big.to_string())).unwrap();
    assert!(frame.len() <= REPLY_BUDGET, "{}", frame.len());
    let reply: Value = serde_json::from_str(&frame).unwrap();
    assert_eq!(reply["type"], "rpc_reply");
    assert_eq!(reply["id"], "req-1");
    assert_eq!(reply["status"], 200);
    assert_eq!(reply["body"]["id"], 42);
    assert_eq!(reply["body"]["result"]["isError"], true);
    let text = reply["body"]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.starts_with("reply too large ("), "{text}");
    assert!(
        text.ends_with("request fewer nodes or a smaller scale"),
        "{text}"
    );
    // A reply inside the budget still round-trips the result unchanged.
    let small = json!({"jsonrpc":"2.0", "id":42, "result":{"content":[]}});
    let frame = rpc_reply("req-1", Some(&small.to_string())).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&frame).unwrap()["body"],
        small
    );
}

#[test]
fn rpc_reply_matches_the_worker_frame_contract() {
    use serde_json::{Value, json};
    // web/worker.mjs webSocketMessage validates exactly this shape: `type` rpc_reply, the
    // envelope `id` as a string, `status` 200 with a JSON-RPC object body (or 202 with a
    // null body for a notification). Status is never optional.
    let reply: Value = serde_json::from_str(
        &rpc_reply("r", Some(r#"{"jsonrpc":"2.0","id":7,"result":{}}"#)).unwrap(),
    )
    .unwrap();
    assert_eq!(reply["type"], "rpc_reply");
    assert!(reply["id"].is_string());
    assert_eq!(reply["status"], 200);
    assert_eq!(reply["body"], json!({"jsonrpc":"2.0", "id":7, "result":{}}));
    let notice: Value = serde_json::from_str(&rpc_reply("r", None).unwrap()).unwrap();
    assert_eq!(notice["status"], 202);
    assert!(notice["body"].is_null());
}

#[test]
fn encode_failure_and_busy_replies_carry_the_callers_id_for_the_worker_match() {
    use serde_json::{Value, json};
    // web/worker.mjs matches reply.body.id against the forwarded request id; the old
    // encode-failure fallback sent a null id, which the worker answers as a 502
    // "Invalid browser reply" instead of the intended error.
    for (frame, status, message) in [
        (
            encode_failed_reply("r", &json!(7)),
            500,
            "Could not encode MCP reply",
        ),
        (
            busy_reply("r", &json!(7)),
            503,
            "Relay is busy; retry later",
        ),
    ] {
        let frame: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(frame["type"], "rpc_reply");
        assert_eq!(frame["id"], "r");
        assert_eq!(frame["status"], status);
        assert_eq!(frame["body"]["jsonrpc"], "2.0");
        assert_eq!(frame["body"]["id"], 7);
        assert_eq!(frame["body"]["error"]["message"], message);
        assert!(frame["body"].get("result").is_none());
    }
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

#[test]
fn a_close_before_welcome_with_1006_is_a_blocked_connection() {
    assert_eq!(closed_message(false, 1006), connect_failed_message());
    assert_eq!(closed_message(true, 1006), close_reason(1006));
    assert_eq!(closed_message(false, 4401), close_reason(4401));
}

use super::*;
use serde_json::{Value, json};
use std::time::Duration;

/// In-flight relay jobs the tab may run at once (`MAX_INFLIGHT` in `mcp_relay.rs`).
const CAP: usize = 8;

fn tabs() -> Vec<Tab> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/structure.fig");
    let doc = Arc::new(figstudio_fig::Document::open(&path).unwrap());
    vec![Tab::new(7, path, doc, None, Duration::ZERO)]
}

fn reveal_call() -> serde_json::Value {
    json!({"jsonrpc":"2.0", "id":"call-1", "method":"tools/call",
        "params":{"name":"reveal_node",
            "arguments":{"nodeId":"1:25", "fileKey":"structure"}}})
}

/// Dispatches one reveal call, lets `before_ui` change the UI state while the worker is
/// blocked, then answers as `McpHost::frame` would (`apply_reveal`) and returns the reply.
fn dispatch_and_apply(
    inbox: &mut Inbox,
    tabs: &mut Vec<Tab>,
    live: Option<u32>,
    before_ui: impl FnOnce(&mut Vec<Tab>),
) -> Value {
    inbox.dispatch(1, "r".to_owned(), reveal_call(), tabs, 0);
    let done = inbox
        .rx
        .recv_timeout(Duration::from_secs(5))
        .expect("a reveal");
    let Done::Reveal {
        generation,
        tab,
        node,
        ack,
        claim,
    } = done
    else {
        panic!("not a reveal");
    };
    assert_eq!(tab, 7);
    before_ui(tabs);
    // `McpHost::frame` claims the reveal before applying it.
    assert!(claim_reveal(&claim));
    let result = apply_reveal(live, generation, tabs, &mut 0, tab, node);
    ack.send(result).unwrap();
    let Done::Reply {
        generation: reply_generation,
        text,
    } = inbox
        .rx
        .recv_timeout(Duration::from_secs(5))
        .expect("a reply")
    else {
        panic!("not a reply");
    };
    assert_eq!(reply_generation, 1);
    serde_json::from_str::<Value>(&text).unwrap()["body"].clone()
}

#[test]
fn a_late_ui_does_not_apply_an_expired_reveal() {
    let mut inbox = Inbox {
        ack_timeout: Duration::from_millis(30),
        ..Default::default()
    };
    let tabs = &mut tabs();
    inbox.dispatch(1, "r".to_owned(), reveal_call(), tabs, 0);
    let Done::Reveal { ack, claim, .. } = inbox
        .rx
        .recv_timeout(Duration::from_secs(5))
        .expect("a reveal")
    else {
        panic!("not a reveal");
    };
    // The worker's bounded wait ends first — the error reply below proves it — so the
    // caller is told the reveal failed.
    let Done::Reply { text, .. } = inbox
        .rx
        .recv_timeout(Duration::from_secs(5))
        .expect("a reply")
    else {
        panic!("not a reply");
    };
    let reply: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(reply["body"]["result"]["isError"], true);
    // A UI frame that runs after that cannot claim it, so it applies nothing — exactly
    // what `McpHost::frame` now does.
    assert!(
        !claim_reveal(&claim),
        "an expired reveal can no longer be claimed"
    );
    let _ = ack.send(Err("the reveal expired before the UI applied it".to_owned()));
    assert_eq!(
        tabs[0].selection, None,
        "an expired reveal must not mutate the UI"
    );
    assert_eq!(tabs[0].fit_node, None);
}

#[test]
fn the_tab_refuses_work_beyond_the_in_flight_cap() {
    let mut inbox = Inbox::default();
    let tabs = &mut tabs();
    let call = |id: usize| json!({"jsonrpc":"2.0", "id":id, "method":"tools/list"});
    for id in 1..=CAP {
        assert_eq!(inbox.dispatch(1, format!("r{id}"), call(id), tabs, 0), None);
    }
    let refused = inbox
        .dispatch(1, format!("r{}", CAP + 1), call(CAP + 1), tabs, 0)
        .expect("the call past the cap must be refused, not stacked");
    let frame: Value = serde_json::from_str(&refused).unwrap();
    assert_eq!(frame["type"], "rpc_reply");
    assert_eq!(frame["id"], format!("r{}", CAP + 1));
    assert_eq!(frame["status"], 503);
    assert_eq!(frame["body"]["id"], json!(CAP + 1));
    assert_eq!(
        frame["body"]["error"]["message"],
        "Relay is busy; retry later"
    );
    let mut replies = 0;
    while let Ok(done) = inbox.rx.recv_timeout(Duration::from_millis(500)) {
        if matches!(done, Done::Reply { .. }) {
            replies += 1;
        }
    }
    assert_eq!(replies, CAP, "only the admitted calls ran");
}

#[test]
fn reveal_waits_for_the_ui_result_before_replying() {
    let tabs = &mut tabs();
    let reply = dispatch_and_apply(&mut Inbox::default(), tabs, Some(1), |_| {});
    assert_eq!(reply["id"], "call-1");
    assert_eq!(reply["result"]["isError"], false);
    // Success means the UI really applied it, not that a frame was queued.
    assert_eq!(tabs[0].selection, Some(Guid(1, 25)));
    assert_eq!(tabs[0].fit_node, Some(Guid(1, 25)));
}

#[test]
fn a_reveal_the_ui_rejects_fails_the_call() {
    let tabs = &mut tabs();
    // The tab closed while the call ran: the UI cannot apply the reveal any more.
    let reply = dispatch_and_apply(&mut Inbox::default(), tabs, Some(1), |t| t.clear());
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(reply["result"]["content"][0]["text"], "tab closed");
}

#[test]
fn stale_or_disconnected_reveals_ack_an_error_without_touching_tabs() {
    let tabs = &mut tabs();
    let node = tabs[0].page;
    assert_eq!(
        apply_reveal(Some(2), 1, tabs, &mut 0, 7, node),
        Err("the tab reconnected; this reveal was dropped".to_owned())
    );
    assert_eq!(
        apply_reveal(None, 1, tabs, &mut 0, 7, node),
        Err("the browser tab disconnected".to_owned())
    );
    assert_eq!((tabs[0].selection, tabs[0].fit_node), (None, None));
}

#[test]
fn a_reveal_never_acked_times_out_as_a_tool_error() {
    let inbox = Inbox::default();
    let env = Snapshot {
        tabs: vec![],
        active: None,
        tx: inbox.tx.clone(),
        generation: 1,
        ack_timeout: Duration::from_millis(20),
    };
    let error = env.reveal(7, Guid(1, 25)).unwrap_err();
    assert!(
        error.to_string().contains("did not apply the reveal"),
        "{error}"
    );
}

#[test]
fn a_finished_call_wakes_the_page_to_send_its_reply() {
    use std::sync::atomic::AtomicUsize;
    static WAKES: AtomicUsize = AtomicUsize::new(0);
    let mut inbox = Inbox {
        wake: || {
            WAKES.fetch_add(1, Ordering::SeqCst);
        },
        ..Default::default()
    };
    let call = json!({"jsonrpc":"2.0", "id":1, "method":"tools/list"});
    assert_eq!(inbox.dispatch(1, "r1".to_owned(), call, &tabs(), 0), None);
    // The wake follows the queued reply: a hidden tab sends it then, not at a frame.
    let deadline = Instant::now() + Duration::from_secs(5);
    while WAKES.load(Ordering::SeqCst) == 0 {
        assert!(
            Instant::now() < deadline,
            "the finished call never woke the page"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let replies = inbox.take_replies();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].0, 1);
    assert_eq!(inbox.pending, 0);
}

#[test]
fn draining_replies_keeps_reveals_for_the_frame() {
    let mut inbox = Inbox {
        ack_timeout: Duration::from_millis(30),
        ..Default::default()
    };
    inbox.dispatch(1, "r".to_owned(), reveal_call(), &tabs(), 0);
    // The reveal expires unacked, so its error reply follows it on the channel.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut replies = Vec::new();
    while replies.is_empty() {
        assert!(Instant::now() < deadline, "no reply");
        replies.extend(inbox.take_replies());
        std::thread::sleep(Duration::from_millis(2));
    }
    let reveals = inbox.take_reveals();
    assert_eq!(reveals.len(), 1, "the reveal stays queued for the UI frame");
    assert!(matches!(reveals[0], Done::Reveal { tab: 7, .. }));
    assert!(inbox.take_reveals().is_empty());
}

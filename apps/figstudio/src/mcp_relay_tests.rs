use super::*;
use serde_json::{Value, json};
use std::time::Duration;

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
    let done = inbox.rx.recv_timeout(Duration::from_secs(5)).expect("a reveal");
    let Done::Reveal {
        generation,
        tab,
        node,
        ack,
    } = done
    else {
        panic!("not a reveal");
    };
    assert_eq!(tab, 7);
    before_ui(tabs);
    let result = apply_reveal(live, generation, tabs, &mut 0, tab, node);
    ack.send(result).unwrap();
    let Done::Reply { text, .. } = inbox.rx.recv_timeout(Duration::from_secs(5)).expect("a reply")
    else {
        panic!("not a reply");
    };
    serde_json::from_str::<Value>(&text).unwrap()["body"].clone()
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

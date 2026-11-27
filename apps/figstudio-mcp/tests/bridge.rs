#[path = "bridge/support.rs"]
mod support;
use figstudio_mcp_core::bridge::TabState;
use rmcp::{ServiceExt, model::CallToolRequestParams, transport::StreamableHttpClientTransport};
use serde_json::{Value, json};
use std::sync::Arc;
use support::*;

#[tokio::test]
async fn upload_selection_reveal_and_disconnect_over_http() {
    let bridge = BridgeTest::start(2 * 1024 * 1024).await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(Arc::<str>::from(
            format!("http://{}/mcp", bridge.addr),
        )))
        .await
        .unwrap();
    let mut socket = bridge.socket("test-pairing-secret").await;
    assert_eq!(
        receive(&mut socket).await,
        ServerMessage::Welcome {
            version: PROTOCOL_VERSION
        }
    );
    upload(&mut socket).await;
    let tabs = call(&client, "list_open_tabs", json!({}))
        .await
        .structured_content
        .unwrap();
    assert_eq!(tabs["tabs"][0]["fileKey"], "structure");
    let file = std::path::PathBuf::from(tabs["tabs"][0]["path"].as_str().unwrap());
    assert!(file.exists());
    let session_dir = file.parent().unwrap().parent().unwrap().to_owned();
    send(
        &mut socket,
        ClientMessage::State {
            tabs: vec![TabState {
                tab: 7,
                page: Some("0:1".into()),
                selection: Some("1:25".into()),
                viewport: Some([0., 0., 500., 500.]),
            }],
            active: Some(7),
        },
    )
    .await;
    send(&mut socket, ClientMessage::End { tab: 99 }).await;
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Error { .. }
    ));
    let selected = call(&client, "get_selection", json!({}))
        .await
        .structured_content
        .unwrap();
    assert_eq!(selected["node"]["nodeId"], "1:25");
    let reveal = client.call_tool(
        CallToolRequestParams::new("reveal_node")
            .with_arguments(json!({"nodeId":"1:25"}).as_object().unwrap().clone()),
    );
    let browser = async {
        match receive(&mut socket).await {
            ServerMessage::Reveal { id, tab, node } => {
                assert_eq!(tab, 7);
                assert_eq!(node, "1:25");
                send(&mut socket, ClientMessage::Reply { id, error: None }).await;
            }
            other => panic!("unexpected frame: {other:?}"),
        }
    };
    let (result, ()) = tokio::join!(reveal, browser);
    assert_ne!(result.unwrap().is_error, Some(true));
    let screenshot = call(&client, "get_screenshot", json!({"nodeId":"1:25"})).await;
    assert_ne!(screenshot.is_error, Some(true));
    assert!(
        screenshot
            .content
            .iter()
            .any(|content| content.as_image().is_some())
    );
    let timed_out = call(&client, "reveal_node", json!({"nodeId":"1:25"})).await;
    assert_eq!(timed_out.is_error, Some(true));
    socket.close(None).await.unwrap();
    for _ in 0..100 {
        let tabs = call(&client, "list_open_tabs", json!({}))
            .await
            .structured_content
            .unwrap();
        if tabs["tabs"].as_array().unwrap().is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        call(&client, "list_open_tabs", json!({}))
            .await
            .structured_content
            .unwrap()["tabs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(!file.exists());
    assert!(!session_dir.exists());
    client.cancel().await.unwrap();
    bridge.finish().await;
}

#[tokio::test]
async fn internal_pages_are_live_pages_and_non_pages_are_still_rejected() {
    let bridge = BridgeTest::start(2 * 1024 * 1024).await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(Arc::<str>::from(
            format!("http://{}/mcp", bridge.addr),
        )))
        .await
        .unwrap();
    let mut socket = bridge.socket("test-pairing-secret").await;
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Welcome { .. }
    ));
    upload(&mut socket).await;
    let doc = figstudio_fig::Document::from_bytes(&fixture()).unwrap();
    for page in [doc.internal_pages[0], doc.pages[0]] {
        send(
            &mut socket,
            ClientMessage::State {
                tabs: vec![TabState {
                    tab: 7,
                    page: Some(page.to_string()),
                    selection: None,
                    viewport: Some([10., 20., 300., 400.]),
                }],
                active: Some(7),
            },
        )
        .await;
        send(&mut socket, ClientMessage::End { tab: 99 }).await;
        assert!(
            matches!(
                receive(&mut socket).await,
                ServerMessage::Error { tab: Some(99), .. }
            ),
            "only the ordering barrier should be rejected"
        );
        let tabs = call(&client, "list_open_tabs", json!({}))
            .await
            .structured_content
            .unwrap();
        assert_eq!(tabs["tabs"][0]["page"]["id"], page.to_string());
        assert!(tabs["tabs"][0]["selection"].is_null());
        let selected = call(&client, "get_selection", json!({}))
            .await
            .structured_content
            .unwrap();
        assert_eq!(selected["page"]["id"], page.to_string());
        assert!(selected["node"].is_null());
    }
    send(
        &mut socket,
        ClientMessage::State {
            tabs: vec![TabState {
                tab: 7,
                page: Some(doc.root.unwrap().to_string()),
                selection: None,
                viewport: None,
            }],
            active: Some(7),
        },
    )
    .await;
    assert!(
        matches!(receive(&mut socket).await, ServerMessage::Error { message, .. } if message == "unknown page")
    );
    let tabs = call(&client, "list_open_tabs", json!({}))
        .await
        .structured_content
        .unwrap();
    assert_eq!(
        tabs["tabs"][0]["page"]["id"],
        doc.pages[0].to_string(),
        "bad state is atomic"
    );
    client.cancel().await.unwrap();
    bridge.finish().await;
}

#[tokio::test]
async fn rejects_unpaired_origins_tokens_uploads_and_bad_messages() {
    let bridge = BridgeTest::start(8).await;
    let mut request = format!("ws://{}/bridge", bridge.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Origin", "https://foreign.invalid".parse().unwrap());
    let error = tokio_tungstenite::connect_async(request).await.unwrap_err();
    assert!(
        matches!(error, tokio_tungstenite::tungstenite::Error::Http(response) if response.status() == 403)
    );
    let mut wrong = bridge.socket("wrong").await;
    assert!(
        matches!(wrong.next().await.unwrap().unwrap(), Message::Close(Some(frame)) if u16::from(frame.code) == 4401)
    );
    let mut socket = bridge.socket("test-pairing-secret").await;
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Welcome { .. }
    ));
    for message in [
        ClientMessage::Open {
            tab: 7,
            name: "structure.fig".into(),
            size: 9,
        },
        ClientMessage::Open {
            tab: 7,
            name: "../escape.fig".into(),
            size: 1,
        },
    ] {
        send(&mut socket, message).await;
        assert!(matches!(
            receive(&mut socket).await,
            ServerMessage::Error { .. }
        ));
    }
    socket.send(Message::Text("not json".into())).await.unwrap();
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Error { .. }
    ));
    send(
        &mut socket,
        ClientMessage::Open {
            tab: 7,
            name: "bad.fig".into(),
            size: 3,
        },
    )
    .await;
    socket
        .send(Message::Binary(vec![1, 2, 3].into()))
        .await
        .unwrap();
    send(&mut socket, ClientMessage::End { tab: 7 }).await;
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Error { .. }
    ));
    socket.close(None).await.unwrap();
    bridge.finish().await;
}

pub async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
    args: Value,
) -> rmcp::model::CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap()
}

/// F6: a tab that gives up on an upload sends `close`; the bridge then drops the partial file,
/// so a later `end` for that tab finds no upload.
#[tokio::test]
async fn close_during_an_upload_drops_it() {
    let bridge = BridgeTest::start(u64::MAX).await;
    let mut socket = bridge.socket("test-pairing-secret").await;
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Welcome { .. }
    ));
    let bytes = fixture();
    send(
        &mut socket,
        ClientMessage::Open {
            tab: 7,
            name: "structure.fig".into(),
            size: bytes.len() as u64,
        },
    )
    .await;
    socket
        .send(Message::Binary(bytes[..1024].to_vec().into()))
        .await
        .unwrap();
    send(&mut socket, ClientMessage::Close { tab: 7 }).await;
    send(&mut socket, ClientMessage::End { tab: 7 }).await;
    let ServerMessage::Error { message, tab } = receive(&mut socket).await else {
        panic!("end after close must fail");
    };
    assert_eq!(message, "no upload in progress");
    assert_eq!(tab, Some(7), "the error names the tab that was closed");
    bridge.finish().await;
}

#[tokio::test]
async fn bridge_omits_unsupported_mutation_and_history_tools() {
    let bridge = BridgeTest::start(2 * 1024 * 1024).await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(Arc::<str>::from(
            format!("http://{}/mcp", bridge.addr),
        )))
        .await
        .unwrap();
    let mut socket = bridge.socket("test-pairing-secret").await;
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Welcome { .. }
    ));
    upload(&mut socket).await;

    let info = client.peer_info().unwrap();
    let instructions = info.instructions.as_deref().unwrap_or_default();
    assert!(instructions.contains("Live mode"), "{instructions}");
    assert!(
        !instructions.contains("the .fig is never changed"),
        "stale write claim remained in instructions: {instructions}"
    );

    let tools = client.list_all_tools().await.unwrap();
    let names: std::collections::BTreeSet<&str> =
        tools.iter().map(|tool| tool.name.as_ref()).collect();
    for unsupported in [
        "apply_edits",
        "undo",
        "redo",
        "get_history",
        "import_image",
        "paste_from_figma",
        "save_copy",
    ] {
        assert!(
            !names.contains(unsupported),
            "read-only --bridge advertised unsupported tool {unsupported}"
        );
    }
    for supported in [
        "list_open_tabs",
        "get_selection",
        "reveal_node",
        "describe_edit_ops",
        "get_editable",
        "copy_for_figma",
        "get_metadata",
        "get_screenshot",
    ] {
        assert!(
            names.contains(supported),
            "--bridge omitted supported tool {supported}"
        );
    }
    assert!(
        client
            .call_tool(
                CallToolRequestParams::new("get_history")
                    .with_arguments(json!({}).as_object().unwrap().clone()),
            )
            .await
            .is_err(),
        "unregistered get_history must fail at protocol level"
    );
    let ops = call(&client, "describe_edit_ops", json!({})).await;
    assert_ne!(ops.is_error, Some(true));
    let editable = call(&client, "get_editable", json!({"nodeId": "1:25"})).await;
    assert_ne!(editable.is_error, Some(true));
    let copied = call(&client, "copy_for_figma", json!({"nodeIds": ["1:25"]})).await;
    assert_ne!(copied.is_error, Some(true));

    client.cancel().await.unwrap();
    bridge.finish().await;
}

#[tokio::test]
async fn unknown_selection_clears_selection_without_poisoning_other_tabs() {
    let bridge = BridgeTest::start(8 * 1024 * 1024).await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(Arc::<str>::from(
            format!("http://{}/mcp", bridge.addr),
        )))
        .await
        .unwrap();
    let mut socket = bridge.socket("test-pairing-secret").await;
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Welcome { .. }
    ));

    upload(&mut socket).await;
    let sections = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/sections.fig"
    ))
    .unwrap();
    send(
        &mut socket,
        ClientMessage::Open {
            tab: 9,
            name: "sections.fig".into(),
            size: sections.len() as u64,
        },
    )
    .await;
    for chunk in sections.chunks(64 * 1024) {
        socket
            .send(Message::Binary(chunk.to_vec().into()))
            .await
            .unwrap();
    }
    send(&mut socket, ClientMessage::End { tab: 9 }).await;

    // Seed tab 7 with a valid selection first so we prove stale selection is cleared, not retained.
    send(
        &mut socket,
        ClientMessage::State {
            tabs: vec![TabState {
                tab: 7,
                page: Some("0:1".into()),
                selection: Some("1:25".into()),
                viewport: Some([0., 0., 500., 500.]),
            }],
            active: Some(7),
        },
    )
    .await;

    // Revision lag: tab 7 reports a newly created layer ID absent from the bridge's snapshot,
    // while tab 9 updates its page/selection and becomes active.
    send(
        &mut socket,
        ClientMessage::State {
            tabs: vec![
                TabState {
                    tab: 7,
                    page: Some("0:1".into()),
                    selection: Some("999:1".into()),
                    viewport: Some([10., 20., 500., 500.]),
                },
                TabState {
                    tab: 9,
                    page: Some("0:1".into()),
                    selection: Some("1:5".into()),
                    viewport: Some([0., 0., 400., 400.]),
                },
            ],
            active: Some(9),
        },
    )
    .await;
    send(&mut socket, ClientMessage::End { tab: 99 }).await;
    assert!(
        matches!(
            receive(&mut socket).await,
            ServerMessage::Error { tab: Some(99), .. }
        ),
        "unknown selection must not wedge or fail the State frame"
    );

    let tabs = call(&client, "list_open_tabs", json!({}))
        .await
        .structured_content
        .unwrap();
    assert!(
        tabs["tabs"][0]["selection"].is_null(),
        "stale selection on tab 7 must be cleared instead of staying 1:25: {tabs}"
    );
    assert_eq!(tabs["tabs"][1]["selection"], "1:5");
    assert_eq!(tabs["tabs"][1]["active"], true);

    let selected = call(&client, "get_selection", json!({}))
        .await
        .structured_content
        .unwrap();
    assert_eq!(selected["tabId"], 9);
    assert_eq!(selected["node"]["nodeId"], "1:5");

    client.cancel().await.unwrap();
    bridge.finish().await;
}

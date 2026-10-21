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

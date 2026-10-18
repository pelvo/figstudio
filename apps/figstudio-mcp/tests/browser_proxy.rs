//! Integration tests for `figstudio-mcp --browser-proxy` (zero-upload Wasm RPC proxy).
use figstudio_mcp::{
    access::HttpAccess,
    proxy::{
        EXEC_PROTOCOL, ProxyConfig, ProxyState, bind_proxy_listener, derive_session_id,
        serve_proxy_http,
    },
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_tungstenite::tungstenite::{ClientRequestBuilder, Message};
use tokio_util::sync::CancellationToken;

async fn start_in_process_proxy() -> (u16, String, std::sync::Arc<ProxyState>, CancellationToken) {
    let listener = bind_proxy_listener(Some("127.0.0.1:0".parse().unwrap()))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = ProxyConfig::new(&["https://figstudio.example.com".to_owned()], false).unwrap();
    let token = config.token.clone();
    let shutdown = CancellationToken::new();
    let state = ProxyState::new(config, port, shutdown.clone());
    let server_state = state.clone();
    tokio::spawn(async move {
        let _ = serve_proxy_http(listener, server_state, HttpAccess::loopback()).await;
    });
    (port, token, state, shutdown)
}

async fn connect_ws(
    port: u16,
    origin: &str,
    protocols: Option<&str>,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::Error,
> {
    let uri = format!("ws://127.0.0.1:{port}/bridge").parse().unwrap();
    let mut builder = ClientRequestBuilder::new(uri).with_header("Origin", origin);
    if let Some(proto) = protocols {
        builder = builder.with_header("Sec-WebSocket-Protocol", proto);
    }
    let (ws, _) = tokio_tungstenite::connect_async(builder).await?;
    Ok(ws)
}

#[tokio::test]
async fn bridge_refuses_foreign_origin_or_bad_subprotocol_and_accepts_valid_proof() {
    let (port, token, _state, shutdown) = start_in_process_proxy().await;
    let good_proto = format!("{EXEC_PROTOCOL}, {token}");
    let bad_proto = format!("{EXEC_PROTOCOL}, {}", "0".repeat(64));

    assert!(
        connect_ws(port, "https://evil.example.com", Some(&good_proto))
            .await
            .is_err()
    );
    assert!(
        connect_ws(port, "https://figstudio.example.com", None)
            .await
            .is_err()
    );
    assert!(
        connect_ws(port, "https://figstudio.example.com", Some(&bad_proto))
            .await
            .is_err()
    );

    let mut ws = connect_ws(port, "https://figstudio.example.com", Some(&good_proto))
        .await
        .expect("valid origin + subprotocol connects");
    ws.send(Message::Text("ping".into())).await.unwrap();
    let pong = ws.next().await.unwrap().unwrap();
    assert_eq!(pong.into_text().unwrap().as_str(), "pong");
    shutdown.cancel();
}

#[tokio::test]
async fn open_figstudio_browser_waits_for_ready_and_forwards_zero_upload_rpc() {
    let (port, token, state, shutdown) = start_in_process_proxy().await;
    let good_proto = format!("{EXEC_PROTOCOL}, {token}");

    // Spawn a call to `open_figstudio_browser` before any browser tab is connected.
    let state_call = state.clone();
    let open_task = tokio::spawn(async move {
        state_call
            .dispatch_rpc(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": {
                        "name": "open_figstudio_browser",
                        "arguments": { "timeoutSeconds": 10 }
                    }
                }),
                Some("__stdio__".to_owned()),
                false,
            )
            .await
            .unwrap()
    });

    tokio::time::sleep(Duration::from_millis(80)).await;
    let mut ws = connect_ws(port, "https://figstudio.example.com", Some(&good_proto))
        .await
        .unwrap();
    ws.send(Message::Text(
        json!({
            "type": "ready",
            "protocol": 2,
            "device": { "id": "dev_local", "label": "Linux · Chrome" },
            "instance": "inst_1",
            "focused": true,
            "docs": [{ "tab": 1, "name": "structure.fig", "fileKey": "structure", "key": "642ee6dfdd5973fe", "pages": 1 }]
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();

    let (status, _, open_reply) = open_task.await.unwrap();
    assert_eq!(status, 200);
    assert_eq!(
        open_reply["result"]["structuredContent"]["connected"],
        json!(true)
    );
    let expected_sid = derive_session_id("dev_local", "inst_1");
    assert_eq!(
        open_reply["result"]["structuredContent"]["sessions"][0]["session"],
        expected_sid
    );

    // Forward a `tools/call` (`get_metadata`) to the connected Wasm tab and check `_meta`.
    let state_rpc = state.clone();
    let call_task = tokio::spawn(async move {
        state_rpc
            .dispatch_rpc(
                json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/call",
                    "params": { "name": "get_metadata", "arguments": { "fileKey": "structure" } }
                }),
                Some("__stdio__".to_owned()),
                false,
            )
            .await
            .unwrap()
    });

    let frame_text = ws.next().await.unwrap().unwrap().into_text().unwrap();
    let frame: Value = serde_json::from_str(&frame_text).unwrap();
    assert_eq!(frame["type"], "rpc");
    let req_id = frame["id"].as_str().unwrap().to_owned();
    ws.send(Message::Text(
        json!({
            "type": "rpc_reply",
            "id": req_id,
            "status": 200,
            "body": {
                "jsonrpc": "2.0",
                "id": 2,
                "result": {
                    "content": [{ "type": "text", "text": "<pages/>" }]
                }
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();

    let (st, _, call_reply) = call_task.await.unwrap();
    assert_eq!(st, 200);
    assert_eq!(
        call_reply["result"]["_meta"]["figstudio/session"],
        expected_sid
    );
    assert_eq!(
        call_reply["result"]["_meta"]["figstudio/device"],
        "Linux · Chrome"
    );
    shutdown.cancel();
}

#[tokio::test]
async fn browser_proxy_binary_speaks_stdio_and_pairs_with_websocket_tab() {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_figstudio-mcp"))
        .args([
            "--browser-proxy",
            "--origin",
            "https://figstudio.example.com",
        ])
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let mut stderr = BufReader::new(child.stderr.take().unwrap()).lines();
    let mut launch_url = None;
    while let Some(line) = stderr.next_line().await.unwrap() {
        if let Some(u) = line.strip_prefix("Launch URL: ") {
            launch_url = Some(u.to_owned());
            break;
        }
    }
    let launch_url = launch_url.expect("Launch URL on stderr");
    let (_, frag) = launch_url.split_once('#').unwrap();
    let mut port = None;
    let mut token = None;
    for part in frag.split('&') {
        if let Some((k, v)) = part.split_once('=') {
            match k {
                "mcpProxyPort" => port = v.parse::<u16>().ok(),
                "mcpProxyToken" => token = Some(v.to_owned()),
                _ => {}
            }
        }
    }
    let (port, token) = (port.unwrap(), token.unwrap());

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();

    // 1. `initialize` over stdio works before any browser tab connects.
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n")
        .await
        .unwrap();
    stdin.flush().await.unwrap();
    let init_line = stdout.next_line().await.unwrap().unwrap();
    let init_val: Value = serde_json::from_str(&init_line).unwrap();
    assert_eq!(init_val["id"], 1);
    assert_eq!(
        init_val["result"]["capabilities"]["tools"]["listChanged"],
        json!(true)
    );

    // 2. Connect a simulated browser tab over WebSocket and send `ready` v2;
    //    verify `notifications/tools/list_changed` is emitted on stdio.
    let proto = format!("{EXEC_PROTOCOL}, {token}");
    let mut ws = connect_ws(port, "https://figstudio.example.com", Some(&proto))
        .await
        .unwrap();
    ws.send(Message::Text(
        json!({
            "type": "ready",
            "protocol": 2,
            "device": { "id": "dev_stdio", "label": "Desktop · Chromium" },
            "instance": "inst_stdio",
            "focused": true,
            "docs": [{ "tab": 1, "name": "design.fig", "fileKey": "design", "key": "0123456789abcdef", "pages": 2 }]
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();

    let notif_line = tokio::time::timeout(Duration::from_secs(5), stdout.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let notif_val: Value = serde_json::from_str(&notif_line).unwrap();
    assert_eq!(notif_val["method"], "notifications/tools/list_changed");

    // 3. Call `list_sessions` over stdio.
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"list_sessions\",\"arguments\":{}}}\n")
        .await
        .unwrap();
    stdin.flush().await.unwrap();
    let ls_line = stdout.next_line().await.unwrap().unwrap();
    let ls_val: Value = serde_json::from_str(&ls_line).unwrap();
    assert_eq!(
        ls_val["result"]["structuredContent"]["sessions"][0]["docs"][0]["fileKey"],
        "design"
    );

    child.kill().await.unwrap();
}

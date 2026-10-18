//! Integration tests for `figstudio-mcp --browser-proxy` (zero-upload Wasm RPC proxy).
use figstudio_mcp::proxy::{
    EXEC_PROTOCOL, ProxyConfig, ProxyState, bind_proxy_listener, derive_session_id,
    parse_browser_entry, serve_proxy_ws,
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
        let _ = serve_proxy_ws(listener, server_state).await;
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

    let state_call = state.clone();
    let open_task = tokio::spawn(async move {
        state_call
            .dispatch_rpc(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "open_figstudio_browser",
                    "arguments": { "timeoutSeconds": 10 }
                }
            }))
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

    let (status, open_reply) = open_task.await.unwrap();
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

    // Even when a tab is already ready, `initialize` and `ping` are answered directly by
    // the proxy with `capabilities.tools.listChanged: true` and without blocking on the tab.
    let (_, init_after) = state
        .dispatch_rpc(json!({"jsonrpc": "2.0", "id": 99, "method": "initialize", "params": {}}))
        .await
        .unwrap();
    assert_eq!(
        init_after["result"]["capabilities"]["tools"]["listChanged"],
        json!(true)
    );
    let (ping_st, ping_after) = state
        .dispatch_rpc(json!({"jsonrpc": "2.0", "id": 100, "method": "ping", "params": {}}))
        .await
        .unwrap();
    assert_eq!(ping_st, 200);
    assert_eq!(ping_after["result"], json!({}));

    // Forward a `tools/call` (`get_metadata`) to the connected Wasm tab and check `_meta`.
    let state_rpc = state.clone();
    let call_task = tokio::spawn(async move {
        state_rpc
            .dispatch_rpc(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": { "name": "get_metadata", "arguments": { "fileKey": "structure" } }
            }))
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

    let (st, call_reply) = call_task.await.unwrap();
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

#[test]
fn browser_entry_parser_handles_percent_s_and_arguments() {
    assert_eq!(
        parse_browser_entry("firefox --new-tab %s", "https://example.com/#x"),
        Some((
            "firefox".to_owned(),
            vec!["--new-tab".to_owned(), "https://example.com/#x".to_owned()]
        ))
    );
    assert_eq!(
        parse_browser_entry("chromium --incognito", "https://example.com/#x"),
        Some((
            "chromium".to_owned(),
            vec![
                "--incognito".to_owned(),
                "https://example.com/#x".to_owned()
            ]
        ))
    );
    assert_eq!(parse_browser_entry("   ", "https://example.com"), None);
}

#[cfg(unix)]
#[tokio::test]
async fn browser_proxy_binary_speaks_stdio_never_logs_token_and_launches_browser() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let url_file = dir.path().join("launched_url.txt");
    let opener = dir.path().join("fake-browser.sh");
    std::fs::write(
        &opener,
        format!("#!/bin/sh\nprintf '%s' \"$2\" > {}\n", url_file.display()),
    )
    .unwrap();
    let mut perms = std::fs::metadata(&opener).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&opener, perms).unwrap();

    // Test colon-separated `BROWSER` with a failing first entry and `%s` substitution,
    // and test 1-line `--origin <URL>` invocation (which implies `--browser-proxy`).
    let browser_env = format!(
        "/nonexistent-figstudio-browser:{} --flag %s",
        opener.display()
    );
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_figstudio-mcp"))
        .args(["--origin", "https://figstudio.example.com"])
        .env("BROWSER", &browser_env)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let mut stderr = BufReader::new(child.stderr.take().unwrap()).lines();
    let banner = stderr.next_line().await.unwrap().unwrap();
    assert!(
        banner.starts_with("Browser proxy: ws://127.0.0.1:") && banner.ends_with("/bridge"),
        "unexpected stderr line: {banner}"
    );
    assert!(
        !banner.contains("mcpProxyToken"),
        "stderr must not leak the secret proxy token: {banner}"
    );

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

    // 2. Call `open_figstudio_browser` over stdio: it invokes `$BROWSER <launch_url>`.
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"open_figstudio_browser\",\"arguments\":{\"timeoutSeconds\":10}}}\n")
        .await
        .unwrap();
    stdin.flush().await.unwrap();

    // Wait for `$BROWSER` to write the launched URL, then connect over WebSocket with its token.
    let mut launch_url = String::new();
    for _ in 0..50 {
        if let Ok(u) = std::fs::read_to_string(&url_file)
            && u.contains("#mcpProxyPort=")
        {
            launch_url = u;
            break;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    let (_, frag) = launch_url
        .split_once('#')
        .expect("launched URL with fragment");
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

    // Read both `notifications/tools/list_changed` and the `open_figstudio_browser` reply.
    let mut got_list_changed = false;
    let mut got_open_reply = false;
    for _ in 0..2 {
        let line = tokio::time::timeout(Duration::from_secs(5), stdout.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let val: Value = serde_json::from_str(&line).unwrap();
        if val.get("method").and_then(Value::as_str) == Some("notifications/tools/list_changed") {
            got_list_changed = true;
        } else if val.get("id") == Some(&json!(2)) {
            assert_eq!(val["result"]["structuredContent"]["connected"], json!(true));
            assert_eq!(val["result"]["structuredContent"]["launched"], json!(true));
            assert_eq!(
                val["result"]["structuredContent"]["sessions"][0]["docs"][0]["fileKey"],
                "design"
            );
            got_open_reply = true;
        }
    }
    assert!(got_list_changed && got_open_reply);

    child.kill().await.unwrap();
    while let Ok(Some(line)) = stderr.next_line().await {
        assert!(
            !line.contains("mcpProxyToken") && !line.contains(&token),
            "stderr leaked proxy token: {line}"
        );
    }
}

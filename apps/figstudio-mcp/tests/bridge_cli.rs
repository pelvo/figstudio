//! The real binary in `--bridge` mode: the printed token and URL pair a socket that acts
//! as the browser, and an MCP client over HTTP sees the uploaded file (docs/WEB-MCP-PLAN.md L4).
#[path = "bridge/support.rs"]
#[expect(dead_code, reason = "shared with the in-process bridge tests")]
mod support;
use rmcp::{ServiceExt, transport::StreamableHttpClientTransport};
use serde_json::json;
use std::sync::Arc;
use support::*;
use tokio::io::{AsyncBufReadExt, BufReader};

/// Reads stderr until the `Web:` line; returns the MCP address and the token.
async fn banner(stderr: tokio::process::ChildStderr) -> (String, String) {
    let mut lines = BufReader::new(stderr).lines();
    let (mut mcp, mut token) = (None, None);
    while let Some(line) = lines.next_line().await.unwrap() {
        if let Some(url) = line.strip_prefix("MCP: ") {
            mcp = Some(url.to_owned());
        } else if let Some((_, t)) = line
            .strip_prefix("Local: ")
            .and_then(|l| l.split_once("#mcp="))
        {
            token = Some(t.to_owned());
        }
        if mcp.is_some() && token.is_some() {
            break;
        }
    }
    (mcp.expect("MCP line"), token.expect("token line"))
}

#[tokio::test]
async fn the_bridge_binary_pairs_a_browser_socket_with_an_mcp_client() {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_figstudio-mcp"))
        .args(["--bridge", "--http=127.0.0.1:0", "--no-system-fonts"])
        .kill_on_drop(true)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let (mcp, token) = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        banner(child.stderr.take().unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(token.len(), 64, "256-bit hex token");
    let addr = mcp
        .strip_prefix("http://")
        .unwrap()
        .strip_suffix("/mcp")
        .unwrap();
    let mut request = format!("ws://{addr}/bridge").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("Origin", "http://localhost:8765".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    send(
        &mut socket,
        ClientMessage::Hello {
            token,
            version: PROTOCOL_VERSION,
        },
    )
    .await;
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Welcome { .. }
    ));
    upload(&mut socket).await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(Arc::<str>::from(
            mcp,
        )))
        .await
        .unwrap();
    let tabs = call(&client, "list_open_tabs", json!({}))
        .await
        .structured_content
        .unwrap();
    assert_eq!(tabs["tabs"][0]["fileKey"], "structure");
    child.kill().await.unwrap();
}

async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
    args: serde_json::Value,
) -> rmcp::model::CallToolResult {
    client
        .call_tool(
            rmcp::model::CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap()
}

/// L5: a non-loopback `--bridge` serves `/mcp` only with the printed bearer token.
#[tokio::test]
async fn a_remote_bridge_requires_the_bearer_token_on_mcp() {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_figstudio-mcp"))
        .args(["--bridge", "--http=0.0.0.0:0", "--allow-any-interface"])
        .args(["--allow-host", "box.example.ts.net", "--no-system-fonts"])
        .kill_on_drop(true)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let (mut bearer, mut port) = (None, None);
    let wait = std::time::Duration::from_secs(20);
    while let Ok(Ok(Some(line))) = tokio::time::timeout(wait, lines.next_line()).await {
        if let Some(token) = line.strip_prefix("Remote access: Authorization: Bearer ") {
            bearer = Some(token.to_owned());
        } else if let Some(url) = line.strip_prefix("MCP: http://0.0.0.0:") {
            port = url.strip_suffix("/mcp").map(str::to_owned);
            break;
        }
    }
    let (bearer, port) = (bearer.expect("bearer line"), port.expect("MCP line"));
    let initialize = async |auth: Option<String>| {
        let mut request = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/mcp"))
            .header("Host", "box.example.ts.net")
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}));
        if let Some(auth) = auth {
            request = request.header("Authorization", auth);
        }
        request.send().await.unwrap().status()
    };
    assert_eq!(initialize(None).await, 401);
    assert_eq!(initialize(Some("Bearer wrong".into())).await, 401);
    assert_eq!(initialize(Some(format!("Bearer {bearer}"))).await, 200);
    child.kill().await.unwrap();
}

/// A non-loopback IPv4 address of this machine, found without sending a packet (the OS picks
/// the source address for a route). `None` on a machine with no network: the test then skips.
fn lan_address() -> Option<std::net::Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

/// F3: a bridge bound to a remote address still pairs the browser on this computer, which
/// connects to `ws://127.0.0.1:<port>/bridge`.
#[tokio::test]
async fn a_remote_bridge_still_pairs_the_browser_on_loopback() {
    let Some(lan) = lan_address() else {
        eprintln!("skipped: no non-loopback IPv4 address on this machine");
        return;
    };
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_figstudio-mcp"))
        .args(["--bridge", &format!("--http={lan}:0"), "--no-system-fonts"])
        .args(["--allow-host", "box.example.ts.net"])
        .kill_on_drop(true)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let (mut token, mut port) = (None, None);
    let wait = std::time::Duration::from_secs(20);
    while let Ok(Ok(Some(line))) = tokio::time::timeout(wait, lines.next_line()).await {
        if let Some(t) = line
            .strip_prefix("Local: ")
            .and_then(|l| l.split_once("#mcp="))
        {
            token = Some(t.1.to_owned());
        } else if let Some(url) = line.strip_prefix("Browser bridge: ws://127.0.0.1:") {
            port = url.strip_suffix("/bridge").map(str::to_owned);
        }
        if token.is_some() && port.is_some() {
            break;
        }
    }
    let (token, port) = (token.expect("token line"), port.expect("bridge line"));
    let mut request = format!("ws://127.0.0.1:{port}/bridge")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Origin", "http://localhost:8765".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    send(
        &mut socket,
        ClientMessage::Hello {
            token,
            version: PROTOCOL_VERSION,
        },
    )
    .await;
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Welcome { .. }
    ));
    child.kill().await.unwrap();
}

//! The real binary in `--bridge` mode: the printed token and URL pair a socket that acts
//! as the browser, and an MCP client over HTTP sees the uploaded file (docs/WEB-MCP-PLAN.md L4).
#[path = "bridge/support.rs"]
#[expect(dead_code, reason = "shared with the in-process bridge tests")]
mod support;
use rmcp::{ServiceExt, transport::StreamableHttpClientTransport};
use serde_json::json;
use std::sync::Arc;
use support::*;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};

/// Reads stderr until the MCP address and local pairing token have arrived.
async fn banner(stderr: &mut (impl AsyncRead + Unpin)) -> (String, String) {
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
async fn banner_keeps_stderr_open_for_later_output() {
    let (mut writer, mut reader) = tokio::io::duplex(1024);
    writer
        .write_all(
            b"MCP: http://127.0.0.1:3845/mcp\nLocal: http://127.0.0.1:8765/#mcp=test-token\n",
        )
        .await
        .unwrap();
    let (mcp, token) = banner(&mut reader).await;
    assert_eq!(mcp, "http://127.0.0.1:3845/mcp");
    assert_eq!(token, "test-token");
    writer
        .write_all(b"Bridge port: 3845 (set the web app URL if not 3845)\n")
        .await
        .expect("reading the banner must not close the child's stderr pipe");
}

#[tokio::test]
async fn the_bridge_binary_pairs_a_browser_socket_with_an_mcp_client() {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_figstudio-mcp"))
        .args(["--bridge", "--http=127.0.0.1:0", "--no-system-fonts"])
        .kill_on_drop(true)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Keep the pipe open: the child still prints banner lines after Local.
    let mut stderr = child.stderr.take().unwrap();
    let (mcp, token) =
        tokio::time::timeout(std::time::Duration::from_secs(20), banner(&mut stderr))
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

/// A non-loopback address of this machine in the given family, found without sending a packet
/// (the OS picks the source address for a route). `None` on a machine without one: the test
/// then prints a skip line.
fn lan_address(v6: bool) -> Option<std::net::IpAddr> {
    let (bind, route) = if v6 {
        ("[::]:0", "[2001:db8::1]:9")
    } else {
        ("0.0.0.0:0", "192.0.2.1:9")
    };
    let socket = std::net::UdpSocket::bind(bind).ok()?;
    socket.connect(route).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// Starts `--bridge` on `lan`, reads the banner and pairs a browser-like socket through
/// `loopback` (the `Browser bridge:` address family under test).
async fn pairs_on_loopback(lan: std::net::IpAddr, loopback: &str) {
    let bind = match lan {
        std::net::IpAddr::V4(ip) => format!("{ip}:0"),
        std::net::IpAddr::V6(ip) => format!("[{ip}]:0"),
    };
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_figstudio-mcp"))
        .args(["--bridge", &format!("--http={bind}"), "--no-system-fonts"])
        .args(["--allow-host", "box.example.ts.net"])
        .kill_on_drop(true)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let (mut token, mut port) = (None, None);
    let wait = std::time::Duration::from_secs(20);
    let wanted = format!("Browser bridge: ws://{loopback}:");
    while let Ok(Ok(Some(line))) = tokio::time::timeout(wait, lines.next_line()).await {
        if let Some(t) = line
            .strip_prefix("Local: ")
            .and_then(|l| l.split_once("#mcp="))
        {
            token = Some(t.1.to_owned());
        } else if let Some(url) = line.strip_prefix(&wanted) {
            port = url.strip_suffix("/bridge").map(str::to_owned);
        }
        if token.is_some() && port.is_some() {
            break;
        }
    }
    let (token, port) = (token.expect("token line"), port.expect("bridge line"));
    let mut request = format!("ws://{loopback}:{port}/bridge")
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

/// F3: a bridge bound to a remote address still pairs the browser on this computer, which
/// connects to `ws://127.0.0.1:<port>/bridge`.
#[tokio::test]
async fn a_remote_bridge_still_pairs_the_browser_on_loopback() {
    let Some(lan) = lan_address(false) else {
        eprintln!(
            "SKIPPED a_remote_bridge_still_pairs_the_browser_on_loopback: no non-loopback IPv4 address"
        );
        return;
    };
    pairs_on_loopback(lan, "127.0.0.1").await;
}

/// R2: an IPv6 bind also listens on `[::1]`, where `localhost` may resolve first.
#[tokio::test]
async fn an_ipv6_remote_bridge_also_pairs_on_ipv6_loopback() {
    let Some(lan) = lan_address(true) else {
        eprintln!(
            "SKIPPED an_ipv6_remote_bridge_also_pairs_on_ipv6_loopback: no non-loopback IPv6 address"
        );
        return;
    };
    pairs_on_loopback(lan, "[::1]").await;
}

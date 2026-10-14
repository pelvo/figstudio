//! Remote agents (docs/WEB-MCP-PLAN.md L5): loopback stays open, a non-loopback bind needs a
//! bearer token and an allowed Host.
use figstudio_mcp::{FigstudioServer, access::HttpAccess, serve_http_with};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ServiceExt, transport::StreamableHttpClientTransport};
use serde_json::json;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig")
}

/// A server on a loopback socket guarded as if bound to `claimed` (a Tailscale-like address).
async fn serve(claimed: &str, hosts: &[String]) -> (SocketAddr, Option<String>, CancellationToken) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let access = HttpAccess::for_bind(claimed.parse().unwrap(), hosts, false).unwrap();
    let token = access.token().map(str::to_owned);
    let stop = CancellationToken::new();
    let server = FigstudioServer::new(vec![fixture()], true);
    let (cancel, task_stop) = (stop.clone(), stop.clone());
    tokio::spawn(async move {
        let _ = serve_http_with(listener, server, task_stop, access).await;
    });
    (addr, token, cancel)
}

async fn initialize(addr: SocketAddr, host: &str, auth: Option<&str>) -> reqwest::StatusCode {
    let mut request = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .header("Host", host)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}));
    if let Some(auth) = auth {
        request = request.header("Authorization", auth);
    }
    request.send().await.unwrap().status()
}

#[tokio::test]
async fn loopback_needs_no_token() {
    let (addr, token, stop) = serve("127.0.0.1:3845", &[]).await;
    assert!(token.is_none());
    assert_eq!(initialize(addr, &addr.to_string(), None).await, 200);
    stop.cancel();
}

#[tokio::test]
async fn a_non_loopback_bind_requires_the_bearer_token() {
    let (addr, token, stop) = serve("100.64.0.1:3845", &[]).await;
    let token = token.unwrap();
    let host = "100.64.0.1:3845";
    for bad in [
        None,
        Some("Bearer wrong"),
        Some(token.as_str()),
        Some("Basic abc"),
        Some("Bearer "),
    ] {
        assert_eq!(initialize(addr, host, bad).await, 401, "{bad:?}");
    }
    // One byte off and one byte short must both fail, not just a different token.
    let off = format!("Bearer {}x", token);
    let short = format!("Bearer {}", &token[..63]);
    let same_length = format!("Bearer {}", "0".repeat(token.len()));
    assert_eq!(
        initialize(addr, host, Some(&same_length)).await,
        401,
        "same length, wrong bytes"
    );
    assert_eq!(initialize(addr, host, Some(&off)).await, 401);
    assert_eq!(initialize(addr, host, Some(&short)).await, 401);
    assert_eq!(
        initialize(addr, host, Some(&format!("Bearer {token}"))).await,
        200
    );
    // A real MCP client with the token lists the tools.
    let config = StreamableHttpClientTransportConfig::with_uri(Arc::<str>::from(format!(
        "http://{addr}/mcp"
    )))
    .auth_header(token);
    let client = ().serve(StreamableHttpClientTransport::from_config(config)).await.unwrap();
    assert!(
        client
            .list_all_tools()
            .await
            .unwrap()
            .iter()
            .any(|t| t.name == "whoami")
    );
    stop.cancel();
}

#[tokio::test]
async fn only_allowed_hosts_pass() {
    let (addr, token, stop) = serve("100.64.0.1:3845", &["box.example.ts.net".into()]).await;
    let auth = format!("Bearer {}", token.unwrap());
    for host in [
        "100.64.0.1:3845",
        "box.example.ts.net",
        "localhost",
        "127.0.0.1:1",
    ] {
        assert_eq!(initialize(addr, host, Some(&auth)).await, 200, "{host}");
    }
    for host in ["evil.example", "100.64.0.2"] {
        assert_eq!(initialize(addr, host, Some(&auth)).await, 403, "{host}");
    }
    stop.cancel();
}

async fn run(
    args: &[&str],
) -> (
    tokio::process::Child,
    tokio::io::Lines<BufReader<tokio::process::ChildStderr>>,
) {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_figstudio-mcp"))
        .args(args)
        .arg(fixture())
        .kill_on_drop(true)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let lines = BufReader::new(child.stderr.take().unwrap()).lines();
    (child, lines)
}

#[tokio::test]
async fn a_wildcard_bind_is_refused_without_the_flag() {
    let (mut child, _) = run(&["--http=0.0.0.0:0"]).await;
    let status = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(!status.success());
}

#[tokio::test]
async fn the_binary_prints_a_token_only_for_remote_binds() {
    let wait = std::time::Duration::from_secs(10);
    let (mut child, mut lines) = run(&["--http=0.0.0.0:0", "--allow-any-interface"]).await;
    let mut text = String::new();
    while let Ok(Ok(Some(line))) = tokio::time::timeout(wait, lines.next_line()).await {
        text.push_str(&line);
        if line.starts_with("Plain HTTP") {
            break;
        }
    }
    assert!(text.contains("Authorization: Bearer "), "{text}");
    child.kill().await.unwrap();
    let (mut local, mut lines) = run(&["--http=127.0.0.1:0"]).await;
    let line = tokio::time::timeout(wait, lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(line.starts_with("listening on http://127.0.0.1:") && !line.contains("Bearer"));
    local.kill().await.unwrap();
}

/// F4: a wildcard bind with the right token still refuses a LAN address as `Host` unless it
/// is named with `--allow-host`; the banner tells the user so.
#[tokio::test]
async fn a_wildcard_bind_refuses_an_unnamed_lan_host_even_with_the_token() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let access = HttpAccess::for_bind("0.0.0.0:3845".parse().unwrap(), &[], true).unwrap();
    let token = access.token().unwrap().to_owned();
    let stop = CancellationToken::new();
    let server = FigstudioServer::new(vec![fixture()], true);
    let task_stop = stop.clone();
    tokio::spawn(async move {
        let _ = serve_http_with(listener, server, task_stop, access).await;
    });
    let auth = format!("Bearer {token}");
    assert_eq!(
        initialize(addr, "192.168.21.37:3845", Some(&auth)).await,
        403
    );
    assert_eq!(initialize(addr, &addr.to_string(), Some(&auth)).await, 200);
    stop.cancel();
}

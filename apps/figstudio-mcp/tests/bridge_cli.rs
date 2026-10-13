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

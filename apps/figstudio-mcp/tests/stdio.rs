use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
use serde_json::json;
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig")
}

#[tokio::test]
async fn stdout_contains_only_one_initialize_reply() {
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_figstudio-mcp"))
        .arg(fixture())
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let request = json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
        "protocolVersion":"2025-03-26", "capabilities":{}, "clientInfo":{"name":"test","version":"1"}
    }});
    let mut input = child.stdin.take().unwrap();
    input
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    drop(input);
    let output = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "unexpected stdout: {stdout}");
    let reply: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["id"], 1);
    assert_eq!(reply["result"]["serverInfo"]["name"], "figstudio-mcp");
}

#[cfg(unix)]
#[tokio::test]
async fn non_loopback_http_warns() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_figstudio-mcp"))
        .args(["--http=0.0.0.0:0"])
        .arg(fixture())
        .kill_on_drop(true)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let warning = tokio::time::timeout(std::time::Duration::from_secs(10), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(warning.contains("non-loopback Host headers"));
    child.kill().await.unwrap();
}

async fn stderr_line(args: &[String]) -> (tokio::process::Child, String) {
    stderr_line_with_env(args, &[]).await
}

async fn stderr_line_with_env(
    args: &[String],
    env: &[(&str, &str)],
) -> (tokio::process::Child, String) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_figstudio-mcp"))
        .args(args)
        .arg(fixture())
        .envs(env.iter().copied())
        .kill_on_drop(true)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let line = tokio::time::timeout(std::time::Duration::from_secs(10), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap_or_default();
    (child, line)
}

#[tokio::test]
async fn bare_http_falls_back_to_the_next_port() {
    // Hold 3845 when it is free, so the server has to move on.
    let held = std::net::TcpListener::bind("127.0.0.1:3845").ok();
    let (mut child, line) = stderr_line(&["--http".into()]).await;
    let port: u16 = line
        .strip_prefix("listening on http://127.0.0.1:")
        .and_then(|rest| rest.strip_suffix("/mcp"))
        .unwrap_or_else(|| panic!("no address on stderr: {line}"))
        .parse()
        .unwrap();
    assert!((3846..=3855).contains(&port), "{port}");
    child.kill().await.unwrap();
    drop(held);
}

#[tokio::test]
async fn listening_line_survives_a_quiet_rust_log() {
    // The port line is program output: RUST_LOG=error must not hide it.
    let (mut child, line) =
        stderr_line_with_env(&["--http".into()], &[("RUST_LOG", "error")]).await;
    assert!(line.starts_with("listening on http://"), "{line}");
    child.kill().await.unwrap();
}

#[tokio::test]
async fn explicit_http_never_falls_back() {
    let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = busy.local_addr().unwrap();
    let (mut child, _) = stderr_line(&[format!("--http={addr}")]).await;
    let status = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(!status.success(), "bound another port instead of failing");
}

#[cfg(unix)]
#[tokio::test]
async fn stdio_sigterm_shuts_down_cleanly() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_figstudio-mcp"))
        .arg(fixture())
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}
    }})).as_bytes()).await.unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    tokio::time::timeout(std::time::Duration::from_secs(10), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    input.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n").await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        tokio::process::Command::new("kill")
            .args(["-TERM", &child.id().unwrap().to_string()])
            .status()
            .await
            .unwrap()
            .success()
    );
    let status = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success(), "SIGTERM was not handled: {status}");
}

#[tokio::test]
async fn stdio_discovers_and_calls_tools() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_figstudio-mcp"));
        command
            .args(["--no-system-fonts", "--assets", "inline"])
            .arg(fixture());
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let info = client.peer_info().unwrap();
        assert_eq!(info.server_info.as_ref().unwrap().name, "figstudio-mcp");
        let instructions = info.instructions.as_ref().unwrap();
        assert!(instructions.starts_with("Offline access to local .fig files only."));
        assert!(instructions.contains("figstudio://<fileKey>?page=0-1&rect=x,y,w,h region link"));
        assert!(instructions.contains("get_nodes_in_region"));
        assert!(
            !instructions.contains("Live mode"),
            "standalone names live mode"
        );
        let tools = client.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 14);
        for name in [
            "get_design_context",
            "whoami",
            "list_files",
            "get_metadata",
            "get_screenshot",
            "get_variable_defs",
            "download_assets",
            "get_figjam",
            "get_context_for_code_connect",
            "get_code_connect_map",
            "add_code_connect_map",
            "search_design_system",
            "get_libraries",
        ] {
            assert!(tools.iter().any(|tool| tool.name == name));
        }
        let result = client
            .call_tool(CallToolRequestParams::new("whoami"))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true));
        assert_eq!(
            result.structured_content,
            Some(json!({"mode":"offline", "server":"figstudio-mcp", "files":["structure"]}))
        );
        assert!(
            client
                .call_tool(CallToolRequestParams::new("unknown"))
                .await
                .is_err()
        );
        let files = client
            .call_tool(CallToolRequestParams::new("list_files"))
            .await
            .unwrap();
        let files = &files.structured_content.unwrap()["files"];
        assert_eq!(files[0]["key"], "structure");
        assert_eq!(files[0]["path"], fixture().to_str().unwrap());
        assert!(files[0]["pages"].is_null());
        let design = client
            .call_tool(
                CallToolRequestParams::new("get_design_context")
                    .with_arguments(json!({"nodeId": "1:25"}).as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_ne!(design.is_error, Some(true));
        let design = serde_json::to_value(&design).unwrap();
        let content = design["content"].as_array().unwrap();
        assert!(
            content[0]["text"]
                .as_str()
                .unwrap()
                .contains(r#"src="structure_1-26.png""#)
        );
        assert_eq!(content.iter().filter(|c| c["type"] == "image").count(), 8);
        client.cancel().await.unwrap();
    })
    .await
    .expect("stdio protocol timed out");
}

#[tokio::test]
async fn trace_logs_do_not_leak_requests_or_layer_names() {
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_figstudio-mcp"))
        .arg("--no-system-fonts")
        .arg(fixture())
        .env("RUST_LOG", "trace")
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let messages = [
        json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
            "protocolVersion":"2025-03-26", "capabilities":{}, "clientInfo":{"name":"test","version":"1"}
        }}),
        json!({"jsonrpc":"2.0", "method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0", "id":2, "method":"tools/call", "params":{
            "name":"get_metadata", "arguments":{"nodeId":"0:1"}
        }}),
    ];
    let mut input = child.stdin.take().unwrap();
    for message in messages {
        input
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
    }
    drop(input);
    let output = tokio::time::timeout(std::time::Duration::from_secs(20), child.wait_with_output())
        .await
        .expect("stdio protocol timed out")
        .unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("Groups"), "layer name on stderr: {stderr}");
    assert!(
        !stderr.contains("arguments"),
        "request body on stderr: {stderr}"
    );
}

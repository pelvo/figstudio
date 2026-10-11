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
        command.arg("--no-system-fonts").arg(fixture());
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let info = client.peer_info().unwrap();
        assert_eq!(info.server_info.as_ref().unwrap().name, "figstudio-mcp");
        assert!(info.instructions.as_ref().unwrap().contains("Offline"));
        let tools = client.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 5);
        for name in ["whoami", "list_files", "get_metadata", "get_screenshot", "get_variable_defs"] {
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
        client.cancel().await.unwrap();
    })
    .await
    .expect("stdio protocol timed out");
}

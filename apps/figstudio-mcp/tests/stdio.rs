use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
use serde_json::json;
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig")
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
        assert_eq!(tools.len(), 2);
        for name in ["whoami", "list_files"] {
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

//! Bound disk and client payloads and preserve permissions across atomic replacement.
use figstudio_mcp::FigstudioServer;
use rmcp::{ServiceExt, model::CallToolRequestParams};
use serde_json::{Value, json};
use std::path::PathBuf;

async fn exercise(case: &str) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("local.fig");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
        &file,
    )
    .unwrap();
    let sidecar = dir.path().join("local.fig.codeconnect.json");
    if case == "disk" {
        let mut data = b"{}".to_vec();
        data.resize(10 * 1024 * 1024 + 1, b' ');
        std::fs::write(&sidecar, data).unwrap();
    }
    #[cfg(unix)]
    if case == "permissions" {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(&sidecar, b"{}").unwrap();
        std::fs::set_permissions(&sidecar, std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    let (a, b) = tokio::io::duplex(1024 * 1024);
    let server = tokio::spawn(FigstudioServer::new(vec![file], true).serve(a));
    let client = ().serve(b).await.unwrap();
    let server = server.await.unwrap().unwrap();
    let mut args =
        json!({"nodeId":"1:37", "componentName":"Button", "source":"button.tsx", "label":"React"});
    match case {
        "source" => args["source"] = json!("x".repeat(4097)),
        "snippet" => args["snippet"] = json!("x".repeat(100_001)),
        "imports" => args["snippetImports"] = json!(["import { Button } from './button'"]),
        _ => (),
    }
    let name = if case == "disk" {
        "get_code_connect_map"
    } else {
        "add_code_connect_map"
    };
    let result = client
        .call_tool(
            CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    match case {
        "disk" | "source" | "snippet" => {
            assert_eq!(result.is_error, Some(true), "{case}");
            if case != "disk" {
                assert!(!sidecar.exists());
            }
        }
        "imports" => assert_eq!(
            result.structured_content.unwrap()["1:37"]["snippetImports"],
            args["snippetImports"]
        ),
        "empty" => {
            let result: Value = result.structured_content.unwrap();
            assert_eq!(result["1:37"].get("snippet"), Some(&Value::Null));
            assert_eq!(result["1:37"]["snippetImports"], json!([]));
        }
        #[cfg(unix)]
        "permissions" | "new_permissions" => {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&sidecar).unwrap().permissions().mode() & 0o777,
                if case == "permissions" { 0o640 } else { 0o644 }
            );
        }
        _ => unreachable!(),
    }
    client.cancel().await.unwrap();
    server.cancel().await.unwrap();
}
#[tokio::test]
async fn oversized_sidecar_is_tool_error() {
    exercise("disk").await;
}
#[tokio::test]
async fn oversized_source_is_rejected_without_write() {
    exercise("source").await;
}
#[tokio::test]
async fn oversized_snippet_is_rejected_without_write() {
    exercise("snippet").await;
}
#[tokio::test]
async fn snippet_imports_input_is_preserved() {
    exercise("imports").await;
}
#[tokio::test]
async fn optional_snippet_keys_are_always_present() {
    exercise("empty").await;
}
#[cfg(unix)]
#[tokio::test]
async fn rewrite_preserves_permissions() {
    exercise("permissions").await;
}
#[cfg(unix)]
#[tokio::test]
async fn new_sidecar_has_public_read_permissions() {
    exercise("new_permissions").await;
}

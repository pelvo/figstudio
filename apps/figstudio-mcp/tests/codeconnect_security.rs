//! The write surface must stay beside the registered file, even with a planted symlink.
#[cfg(unix)]
#[tokio::test]
async fn sidecar_symlink_cannot_write_outside_file_directory() {
    use figstudio_mcp::FigstudioServer;
    use rmcp::{ServiceExt, model::CallToolRequestParams};
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let protected = outside.path().join("protected.json");
    std::fs::write(&protected, b"{}").unwrap();
    let file = dir.path().join("local.fig");
    std::fs::copy(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/structure.fig"),
        &file,
    )
    .unwrap();
    let sidecar = dir.path().join("local.fig.codeconnect.json");
    std::os::unix::fs::symlink(&protected, &sidecar).unwrap();
    let (a, b) = tokio::io::duplex(65536);
    let server = tokio::spawn(FigstudioServer::new(vec![file], true).serve(a));
    let client = ().serve(b).await.unwrap();
    let server = server.await.unwrap().unwrap();
    for name in ["get_code_connect_map", "add_code_connect_map"] {
        let args = json!({"nodeId":"1:37", "componentName":"Button", "source":"../../protected.json", "label":"React"});
        let result = client
            .call_tool(
                CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
    }
    assert_eq!(std::fs::read(&protected).unwrap(), b"{}");
    assert!(
        std::fs::symlink_metadata(&sidecar)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    client.cancel().await.unwrap();
    server.cancel().await.unwrap();
}

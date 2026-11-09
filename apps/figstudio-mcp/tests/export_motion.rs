//! `export_motion`: an animated layer as a file, inline or in the assets folder.

use figstudio_mcp::FigstudioServer;
use figstudio_mcp::tools::design::Assets;
use rmcp::{ServiceExt, model::CallToolRequestParams};
use serde_json::json;
use std::path::PathBuf;

fn motion() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/motion.fig")
}

async fn call(server: FigstudioServer, args: serde_json::Value) -> rmcp::model::CallToolResult {
    let (a, b) = tokio::io::duplex(1 << 22);
    let server = tokio::spawn(server.serve(a));
    let client = ().serve(b).await.unwrap();
    let _server = server.await.unwrap().unwrap();
    client
        .call_tool(
            CallToolRequestParams::new("export_motion")
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn a_gif_comes_back_as_an_image_and_an_mp4_as_a_resource() {
    let inline = || FigstudioServer::new(vec![motion()], true).with_assets(Assets::Inline);
    let gif = call(
        inline(),
        json!({"nodeId": "1:1106", "format": "gif", "scale": 0.25, "fps": 10}),
    )
    .await;
    assert_ne!(gif.is_error, Some(true), "{gif:?}");
    assert!(
        gif.content
            .iter()
            .any(|c| c.as_image().is_some_and(|i| i.mime_type == "image/gif"))
    );
    let mp4 = call(
        inline(),
        json!({"nodeId": "1:1106", "scale": 0.25, "fps": 10}),
    )
    .await;
    assert_ne!(mp4.is_error, Some(true), "{mp4:?}");
    assert!(
        mp4.content.iter().any(|c| c.as_resource().is_some()),
        "{mp4:?}"
    );
}

#[tokio::test]
async fn with_an_assets_folder_the_file_is_written_there() {
    let dir = tempfile::tempdir().unwrap();
    let server = FigstudioServer::new(vec![motion()], true)
        .with_assets(Assets::dir(dir.path().to_path_buf()));
    let done = call(
        server,
        json!({"nodeId": "1:1106", "format": "webm", "scale": 0.25, "fps": 10}),
    )
    .await;
    assert_ne!(done.is_error, Some(true), "{done:?}");
    let written: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].extension().unwrap(), "webm");
    // A still layer is an error result, not a crash.
    let server = FigstudioServer::new(vec![motion()], true).with_assets(Assets::Inline);
    let still = call(server, json!({"nodeId": "0:1"})).await;
    assert_eq!(still.is_error, Some(true));
}

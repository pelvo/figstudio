//! Region links through the protocol: `get_screenshot` and `get_nodes_in_region`.
use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_mcp::FigstudioServer;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
};
use serde_json::{Value, json};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

type Client = rmcp::service::RunningService<rmcp::RoleClient, ()>;

async fn call(client: &Client, tool: &'static str, args: Value) -> CallToolResult {
    let request =
        CallToolRequestParams::new(tool).with_arguments(args.as_object().unwrap().clone());
    client.call_tool(request).await.unwrap()
}

async fn run(check: impl AsyncFnOnce(&Client)) {
    let (client_io, server_io) = tokio::io::duplex(1 << 22);
    let paths = vec![fixture("structure.fig"), fixture("sections.fig")];
    let task = tokio::spawn(async move {
        FigstudioServer::new(paths, true)
            .serve(server_io)
            .await
            .unwrap()
            .waiting()
            .await
    });
    let client = ().serve(client_io).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(60), check(&client))
        .await
        .expect("timed out");
    client.cancel().await.unwrap();
    task.await.unwrap().unwrap();
}

fn text(result: &CallToolResult) -> String {
    let v = serde_json::to_value(result).unwrap();
    v["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "text")
        .unwrap()["text"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn png_size(result: &CallToolResult) -> (u32, u32) {
    assert_ne!(result.is_error, Some(true), "{}", text(result));
    let v = serde_json::to_value(result).unwrap();
    let image = v["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "image")
        .unwrap();
    let bytes = STANDARD.decode(image["data"].as_str().unwrap()).unwrap();
    let png = image::load_from_memory(&bytes).unwrap();
    (png.width(), png.height())
}

#[tokio::test]
async fn a_region_link_screenshot_picks_its_file_and_has_the_rect_size() {
    run(async |client| {
        let link = "figstudio://structure?page=0-1&rect=10,20,120,60.5";
        let shot = call(client, "get_screenshot", json!({"nodeId": link})).await;
        assert_eq!(png_size(&shot), (120, 61));
        let twice = call(
            client,
            "get_screenshot",
            json!({"nodeId": link, "scale": 2}),
        )
        .await;
        assert_eq!(png_size(&twice), (240, 121));
        // Both files are open, yet the link needs no fileKey; an unknown file fails the call.
        let other = call(
            client,
            "get_screenshot",
            json!({"nodeId": "figstudio://nope?page=0-1&rect=0,0,5,5"}),
        )
        .await;
        assert_eq!(other.is_error, Some(true));
    })
    .await;
}

#[tokio::test]
async fn a_bad_rect_is_a_tool_error() {
    run(async |client| {
        for link in [
            "figstudio://structure?page=0-1&rect=0,0,0,5",
            "figstudio://structure?page=0-1&rect=0,0,5",
            "figstudio://structure?page=0-1&rect=nan,0,5,5",
            "figstudio://structure?page=9-9&rect=0,0,5,5",
        ] {
            let result = call(client, "get_screenshot", json!({"nodeId": link})).await;
            assert_eq!(result.is_error, Some(true), "{link}");
        }
        let nodes = call(
            client,
            "get_nodes_in_region",
            json!({"fileKey": "structure", "page": "0:1", "rect": [0, 0, -1, 5]}),
        )
        .await;
        assert_eq!(nodes.is_error, Some(true));
        assert!(text(&nodes).contains("rect must be"), "{}", text(&nodes));
    })
    .await;
}

#[tokio::test]
async fn get_nodes_in_region_lists_nodes_for_a_link_and_for_page_and_rect() {
    run(async |client| {
        let rect = json!([-500, -500, 5000, 5000]);
        let by_rect = call(
            client,
            "get_nodes_in_region",
            json!({"fileKey": "structure", "page": "0:1", "rect": rect, "limit": 3}),
        )
        .await;
        let v = by_rect.structured_content.clone().unwrap();
        assert_eq!(v["nodes"].as_array().unwrap().len(), 3);
        assert_eq!(v["truncated"], true);
        assert!(v["total"].as_u64().unwrap() > 3);
        assert!(v["nodes"][0]["nodeId"].is_string());
        let by_link = call(
            client,
            "get_nodes_in_region",
            json!({"region": "figstudio://structure?page=0-1&rect=-500,-500,5000,5000", "limit": 3}),
        )
        .await;
        assert_eq!(by_link.structured_content, by_rect.structured_content);
    })
    .await;
}

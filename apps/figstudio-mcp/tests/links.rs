//! Links as node IDs: `figstudio://` picks the file, figma.com links pick it by key or slug.
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

fn text(result: &CallToolResult) -> String {
    serde_json::to_value(result).unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    tool: &'static str,
    args: Value,
) -> CallToolResult {
    let request =
        CallToolRequestParams::new(tool).with_arguments(args.as_object().unwrap().clone());
    client.call_tool(request).await.unwrap()
}

async fn run(
    paths: Vec<PathBuf>,
    check: impl AsyncFnOnce(&rmcp::service::RunningService<rmcp::RoleClient, ()>),
) {
    let (client_io, server_io) = tokio::io::duplex(65536);
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

async fn page_names(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    args: Value,
) -> String {
    let result = call(client, "get_metadata", args).await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    text(&result)
}

#[tokio::test]
async fn a_figstudio_link_selects_its_file_without_file_key() {
    run(
        vec![fixture("structure.fig"), fixture("sections.fig")],
        async |client| {
            // Both files have a page 0:1 named alike; only the page contents tell them apart.
            let structure = page_names(client, json!({"fileKey":"structure","nodeId":"0-1"})).await;
            let sections = page_names(client, json!({"fileKey":"sections","nodeId":"0-1"})).await;
            assert_ne!(structure, sections);
            for (key, expected) in [("structure", &structure), ("sections", &sections)] {
                let link = format!("figstudio://{key}?node-id=0-1");
                assert_eq!(
                    &page_names(client, json!({"nodeId": link})).await,
                    expected,
                    "{key}"
                );
            }
            // An explicit fileKey wins over the link's file.
            let wins = page_names(
                client,
                json!({"fileKey":"sections","nodeId":"figstudio://structure?node-id=0-1"}),
            )
            .await;
            assert_eq!(
                wins,
                page_names(client, json!({"fileKey":"sections","nodeId":"0-1"})).await
            );
            // An unknown figstudio:// file is a tool error, never a fallback.
            let result = call(
                client,
                "get_metadata",
                json!({"nodeId":"figstudio://nope?node-id=0-1"}),
            )
            .await;
            assert_eq!(result.is_error, Some(true));
            assert!(text(&result).contains("fileKey"), "{}", text(&result));
        },
    )
    .await;
}

#[tokio::test]
async fn figma_links_select_a_file_by_key_or_slug_and_otherwise_fall_back() {
    run(
        vec![fixture("structure.fig"), fixture("sections.fig")],
        async |client| {
            let sections = page_names(client, json!({"fileKey":"sections","nodeId":"0-1"})).await;
            for url in [
                "https://www.figma.com/design/sections/Any?node-id=0-1",
                "https://www.figma.com/design/ZZZ/sections?node-id=0-1",
            ] {
                assert_eq!(
                    page_names(client, json!({"nodeId": url})).await,
                    sections,
                    "{url}"
                );
            }
            // Unmatched key with two files open: today's rule, so a file-key error.
            let result = call(
                client,
                "get_metadata",
                json!({"nodeId":"https://www.figma.com/design/ZZZ/zzz?node-id=0-1"}),
            )
            .await;
            assert_eq!(result.is_error, Some(true));
        },
    )
    .await;
    // One file: an unmatched figma.com key is ignored, as before.
    run(vec![fixture("structure.fig")], async |client| {
        let url = "https://www.figma.com/design/ZZZ/zzz?node-id=0-1";
        let by_url = page_names(client, json!({"nodeId": url})).await;
        assert_eq!(by_url, page_names(client, json!({"nodeId":"0-1"})).await);
    })
    .await;
}

#[tokio::test]
async fn other_tools_take_the_file_from_a_link_too() {
    run(
        vec![fixture("structure.fig"), fixture("sections.fig")],
        async |client| {
            let result = call(
                client,
                "get_variable_defs",
                json!({"nodeId":"figstudio://sections?node-id=0-1"}),
            )
            .await;
            assert_ne!(result.is_error, Some(true), "{}", text(&result));
            let result = call(
                client,
                "get_screenshot",
                json!({"nodeId":"figstudio://sections?node-id=0-1"}),
            )
            .await;
            assert_ne!(result.is_error, Some(true), "{}", text(&result));
            let result = call(
                client,
                "download_assets",
                json!({"nodeIds":["figstudio://sections?node-id=0-1"]}),
            )
            .await;
            assert_ne!(result.is_error, Some(true), "{}", text(&result));
        },
    )
    .await;
}

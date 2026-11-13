//! End-to-end tests for the MCP Editing Suite (`describe_edit_ops`, `get_editable`,
//! `apply_edits`, `undo`, `redo`, `get_history`, `copy_for_figma`, `paste_from_figma`,
//! `import_image`, and `save_copy`).

use std::path::PathBuf;

use figstudio_fig::Document;
use figstudio_mcp::FigstudioServer;
use rmcp::{ServiceExt, model::CallToolRequestParams};
use serde_json::{Value, json};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[tokio::test]
async fn mcp_editing_workflow_modifies_renders_copies_pastes_undoes_and_saves() {
    let src_path = fixture("structure.fig");
    let orig_bytes = std::fs::read(&src_path).unwrap();

    let (a, b) = tokio::io::duplex(1 << 20);
    let server = FigstudioServer::new(vec![src_path.clone()], true);
    let server_task = tokio::spawn(server.serve(a));
    let client = ().serve(b).await.unwrap();

    let call = |name: &'static str, args: Value| {
        let c = client.clone();
        async move {
            let params = CallToolRequestParams::new(name)
                .with_arguments(args.as_object().cloned().unwrap_or_default());
            let res = c.call_tool(params).await.expect(name);
            serde_json::to_value(res).unwrap()
        }
    };

    // 1. Describe ops
    let desc = call("describe_edit_ops", json!({})).await;
    assert_ne!(desc["isError"], json!(true));
    assert!(
        desc["structuredContent"]["operations"]
            .as_array()
            .unwrap()
            .len()
            >= 25
    );

    // 2. Discover page ID via metadata, then create a new Auto-Layout Frame + Text
    let doc = Document::open(&src_path).unwrap();
    let page_id = doc.pages[0].to_string();

    let applied = call(
        "apply_edits",
        json!({
            "label": "Create MCP Banner",
            "ops": [
                {
                    "op": "create_shape",
                    "kind": "FRAME",
                    "parent": page_id,
                    "name": "MCP Banner",
                    "x": 100.0,
                    "y": 100.0,
                    "width": 280.0,
                    "height": 64.0,
                    "corner_radius": 10.0,
                    "fill_hex": "#1E293B"
                }
            ]
        }),
    )
    .await;
    assert_ne!(
        applied["isError"],
        json!(true),
        "apply_edits failed: {applied}"
    );
    let banner_id = applied["structuredContent"]["created_ids"][0]
        .as_str()
        .unwrap()
        .to_string();

    // 3. Inspect newly created node via `get_editable` and render via `get_screenshot`!
    let editable = call("get_editable", json!({ "nodeId": banner_id })).await;
    assert_eq!(editable["structuredContent"]["name"], json!("MCP Banner"));
    assert_eq!(editable["structuredContent"]["width"], json!(280.0));

    let shot = call("get_screenshot", json!({ "nodeId": banner_id })).await;
    assert_ne!(
        shot["isError"],
        json!(true),
        "screenshot of new node failed: {shot}"
    );

    // 4. Copy for Figma -> Paste back -> verify history -> Save copy -> Undo!
    let copied = call("copy_for_figma", json!({ "nodeIds": [banner_id] })).await;
    let html = copied["structuredContent"]["html"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(html.contains("data-buffer=\"<!--(figma)"));

    let pasted = call(
        "paste_from_figma",
        json!({ "html": html, "parentId": page_id, "dx": 30.0, "dy": 30.0 }),
    )
    .await;
    assert_eq!(
        pasted["structuredContent"]["pastedRootIds"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let tmp_dir = tempfile::tempdir().unwrap();
    let out_fig = tmp_dir.path().join("edited_out.fig");
    let saved = call("save_copy", json!({ "outPath": out_fig.to_string_lossy() })).await;
    assert_ne!(saved["isError"], json!(true), "save_copy failed: {saved}");
    let reopened = Document::open(&out_fig).unwrap();
    assert!(
        reopened
            .nodes
            .iter()
            .filter(|n| n.name == "MCP Banner")
            .count()
            >= 2
    );

    // Original source file on disk was NEVER modified!
    assert_eq!(std::fs::read(&src_path).unwrap(), orig_bytes);

    let hist = call("get_history", json!({})).await;
    assert!(hist["structuredContent"]["undo"].as_array().unwrap().len() >= 2);

    let u1 = call("undo", json!({})).await;
    assert_ne!(u1["isError"], json!(true));
    let r1 = call("redo", json!({})).await;
    assert_ne!(r1["isError"], json!(true));

    client.cancel().await.unwrap();
    let _ = server_task.await.unwrap();
}

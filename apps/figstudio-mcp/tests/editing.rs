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
    let tmp_dir = tempfile::tempdir().unwrap();
    let src_path = tmp_dir.path().join("src.fig");
    std::fs::copy(fixture("structure.fig"), &src_path).unwrap();
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

    // T19: read-only `get_history` before any edit reports `unsaved: false` cleanly.
    let hist0 = call("get_history", json!({})).await;
    assert_ne!(hist0["isError"], json!(true));
    assert_eq!(hist0["structuredContent"]["unsaved"], json!(false));

    // T6: multibyte invalid `fill_hex` must return a clean validation error, NOT `document loading task failed`!
    let doc = Document::open(&src_path).unwrap();
    let page_id = doc.pages[0].to_string();
    let bad_hex = call(
        "apply_edits",
        json!({
            "ops": [{
                "op": "create_shape",
                "kind": "ROUNDED_RECTANGLE",
                "parent": page_id,
                "x": 0.0, "y": 0.0, "width": 40.0, "height": 40.0,
                "fill_hex": "aé123"
            }]
        }),
    )
    .await;
    assert_eq!(bad_hex["isError"], json!(true));
    let err_txt = serde_json::to_string(&bad_hex).unwrap();
    assert!(
        !err_txt.contains("document loading task failed"),
        "worker panicked instead of returning validation error: {err_txt}"
    );

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

    // 2. Create a new Frame + a newly created Text node ("Hello edit" at 24px) + Star!
    let applied = call(
        "apply_edits",
        json!({
            "label": "Create MCP Banner + Text",
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
                },
                {
                    "op": "create_text",
                    "parent": page_id,
                    "x": 116.0,
                    "y": 116.0,
                    "characters": "Hello edit",
                    "font_size": 24.0,
                    "fill_hex": "#111111"
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
    assert_eq!(applied["structuredContent"]["unsaved"], json!(true));
    let banner_id = applied["structuredContent"]["created_ids"][0]
        .as_str()
        .unwrap()
        .to_string();
    let text_id = applied["structuredContent"]["created_ids"][1]
        .as_str()
        .unwrap()
        .to_string();

    // T3: Newly created text node MUST render visible pixels via `get_screenshot` AND after `save_copy` reopen!
    let text_shot = call("get_screenshot", json!({ "nodeId": text_id })).await;
    assert_ne!(
        text_shot["isError"],
        json!(true),
        "newly created text node failed to render: {text_shot}"
    );

    // 4. Copy for Figma -> Paste back -> verify history -> Save copy checks!
    let copied = call("copy_for_figma", json!({ "nodeIds": [banner_id] })).await;
    let html = copied["structuredContent"]["html"]
        .as_str()
        .unwrap()
        .to_string();
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

    // T1a: Reject overwriting source file (even with overwrite: true)!
    let over_src = call(
        "save_copy",
        json!({ "outPath": src_path.to_string_lossy(), "overwrite": true }),
    )
    .await;
    assert_eq!(
        over_src["isError"],
        json!(true),
        "save_copy must reject overwriting source file!"
    );

    // T1b: Reject non-.fig extension!
    let non_fig = call(
        "save_copy",
        json!({ "outPath": tmp_dir.path().join("evil.txt").to_string_lossy() }),
    )
    .await;
    assert_eq!(
        non_fig["isError"],
        json!(true),
        "save_copy must reject non-.fig path!"
    );

    // T1c: Reject `..` traversal / escaping outside the source file's folder!
    let traversal = call("save_copy", json!({ "outPath": "../escape.fig" })).await;
    assert_eq!(
        traversal["isError"],
        json!(true),
        "save_copy must reject .. traversal!"
    );
    let other_dir = tempfile::tempdir().unwrap();
    let outside = call(
        "save_copy",
        json!({ "outPath": other_dir.path().join("outside.fig").to_string_lossy() }),
    )
    .await;
    assert_eq!(
        outside["isError"],
        json!(true),
        "save_copy must reject writing outside the registered file's directory!"
    );

    // Valid save_copy to `edited_out.fig` next to `src.fig` succeeds and marks `unsaved: false`!
    let out_fig = tmp_dir.path().join("edited_out.fig");
    let saved = call("save_copy", json!({ "outPath": out_fig.to_string_lossy() })).await;
    assert_ne!(saved["isError"], json!(true), "save_copy failed: {saved}");
    assert_eq!(saved["structuredContent"]["unsaved"], json!(false));

    // T1d: Saving again over existing `edited_out.fig` WITHOUT `overwrite: true` MUST fail!
    let dup_save = call("save_copy", json!({ "outPath": out_fig.to_string_lossy() })).await;
    assert_eq!(
        dup_save["isError"],
        json!(true),
        "save_copy must refuse replacing existing file without overwrite: true"
    );

    // T1e: Saving over existing `edited_out.fig` WITH `overwrite: true` succeeds!
    let ok_overwrite = call(
        "save_copy",
        json!({ "outPath": out_fig.to_string_lossy(), "overwrite": true }),
    )
    .await;
    assert_ne!(
        ok_overwrite["isError"],
        json!(true),
        "save_copy with overwrite: true failed: {ok_overwrite}"
    );

    // T16/T19: Undo after save marks `unsaved: true`; Redo back to saved state marks `unsaved: false`!
    let u_after_save = call("undo", json!({})).await;
    assert_eq!(u_after_save["structuredContent"]["unsaved"], json!(true));
    let r_after_save = call("redo", json!({})).await;
    assert_eq!(r_after_save["structuredContent"]["unsaved"], json!(false));

    // T3: Reopen saved `.fig` and render newly created text node directly via `figstudio_render`!
    let reopened = Document::open(&out_fig).unwrap();
    let parsed_text_guid: figstudio_fig::Guid = text_id.parse().unwrap();
    let mut renderer = figstudio_render::Renderer::new();
    let frame = renderer
        .render_export_complete(&reopened, parsed_text_guid, 1.0, None, |_| None)
        .expect("newly created text node must render visible pixels, not None!");
    let non_transparent_pixels = frame
        .pixmap
        .data_as_u8_slice()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[3] > 16)
        .count();
    assert!(
        non_transparent_pixels >= 20,
        "newly created text node drew {non_transparent_pixels} visible pixels!"
    );

    // Original source file on disk was NEVER modified!
    assert_eq!(std::fs::read(&src_path).unwrap(), orig_bytes);

    client.cancel().await.unwrap();
    let _ = server_task.await.unwrap();
}

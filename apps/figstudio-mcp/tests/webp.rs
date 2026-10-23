//! Lossless format selection survives the MCP transport, inline blocks and file output.
use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_mcp::{FigstudioServer, tools::design::Assets};
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
};
use serde_json::{Value, json};

async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
    args: Value,
) -> CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn lossless_formats_cover_all_image_tools() {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/structure.fig");
    let dir = tempfile::tempdir().unwrap();
    let mut doc = figstudio_fig::Document::open(&fixture).unwrap();
    let mut nodes = doc.nodes.clone();
    let node = nodes.iter_mut().find(|n| n.kind == "FRAME").unwrap();
    node.data
        .set("name", figstudio_fig::Val::Str("format-example".into()));
    let node_id = node.id.to_string();
    let mut paint = figstudio_fig::Obj::new();
    paint.insert("type", figstudio_fig::Val::Enum("SOLID"));
    paint.insert(
        "color",
        figstudio_fig::Val::Obj(figstudio_fig::Obj::from([
            ("r", figstudio_fig::Val::Float(1.0)),
            ("g", figstudio_fig::Val::Float(0.0)),
            ("b", figstudio_fig::Val::Float(1.0)),
            ("a", figstudio_fig::Val::Float(1.0)),
        ])),
    );
    node.data.set(
        "fillPaints",
        figstudio_fig::Val::Arr(vec![figstudio_fig::Val::Obj(paint)]),
    );
    node.data.set("type", figstudio_fig::Val::Enum("STICKY"));
    *node = figstudio_fig::Node::from_data(node.data.clone()).unwrap();
    doc.set_nodes(nodes).unwrap();
    let changed = dir.path().join("changed.fig");
    doc.save(&changed).unwrap();
    let assets = dir.path().join("assets");
    let vector = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/vector.fig");
    let vector_page = figstudio_fig::Document::open(&vector).unwrap().pages[0].to_string();
    let server = FigstudioServer::new(vec![fixture, changed, vector], true)
        .with_assets(Assets::dir(assets.clone()));
    let (a, b) = tokio::io::duplex(65536);
    let task = tokio::spawn(server.serve(a));
    let client = ().serve(b).await.unwrap();
    let service = task.await.unwrap().unwrap();
    let tools = client.list_tools(Default::default()).await.unwrap();
    for (tool, base) in [
        (
            "get_screenshot",
            json!({"fileKey":"structure", "nodeId":node_id}),
        ),
        (
            "download_assets",
            json!({"fileKey":"structure", "nodeIds":[node_id]}),
        ),
        (
            "get_design_context",
            json!({"fileKey":"vector", "nodeId":vector_page}),
        ),
        (
            "get_figjam",
            json!({"fileKey":"changed", "nodeId":node_id,"includeImages":true}),
        ),
        (
            "compare_files",
            json!({"oldFile":"structure","newFile":"changed","includeRender":true}),
        ),
    ] {
        let schema = tools.tools.iter().find(|t| t.name == tool).unwrap();
        let schema = serde_json::to_value(&schema.input_schema).unwrap();
        assert!(schema["properties"].get("format").is_some(), "{tool}");
        for invalid in ["jxl", "jpg", "WEBP", ""] {
            if invalid == "jpg" && tool == "download_assets" {
                let mut args = base.clone();
                args["format"] = json!("jpg");
                let reply = call(&client, tool, args).await;
                assert_ne!(reply.is_error, Some(true), "{reply:?}");
                let json = serde_json::to_value(&reply).unwrap();
                let image = json["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|b| b["type"] == "image")
                    .unwrap();
                assert_eq!(image["mimeType"], "image/jpeg");
                continue;
            }
            let mut args = base.clone();
            args["format"] = json!(invalid);
            let reply = call(&client, tool, args).await;
            assert_eq!(reply.is_error, Some(true), "{tool}: {reply:?}");
        }
        for format in ["webp", "png"] {
            let mut args = base.clone();
            args["format"] = json!(format);
            let reply = call(&client, tool, args).await;
            assert_ne!(reply.is_error, Some(true), "{tool}: {reply:?}");
            let json = serde_json::to_value(&reply).unwrap();
            let mut images = 0;
            for block in json["content"].as_array().unwrap() {
                if block["type"] != "image" {
                    continue;
                }
                images += 1;
                assert_eq!(block["mimeType"], format!("image/{format}"));
                let bytes = STANDARD.decode(block["data"].as_str().unwrap()).unwrap();
                assert_eq!(
                    image::guess_format(&bytes).unwrap(),
                    if format == "webp" {
                        image::ImageFormat::WebP
                    } else {
                        image::ImageFormat::Png
                    }
                );
                assert!(image::load_from_memory(&bytes).unwrap().width() > 0);
            }
            if matches!(tool, "get_screenshot" | "download_assets" | "get_figjam") {
                assert_eq!(images, 1, "{tool}");
            }
            if tool == "get_design_context" {
                let text = serde_json::to_string(&reply).unwrap();
                let paths: Vec<_> = std::fs::read_dir(&assets)
                    .unwrap()
                    .map(|p| p.unwrap().path())
                    .filter(|p| p.extension().is_some_and(|e| e == format))
                    .collect();
                assert!(!paths.is_empty());
                // Every `src` in the generated code names a written asset of this format.
                let code = json["content"][0]["text"].as_str().unwrap();
                let srcs: Vec<_> = code
                    .split("src=\"")
                    .skip(1)
                    .map(|rest| rest.split('"').next().unwrap())
                    .collect();
                assert!(!srcs.is_empty(), "{code}");
                for src in srcs {
                    assert!(src.ends_with(&format!(".{format}")), "{src}");
                    assert!(std::path::Path::new(src).is_file(), "{src}");
                }
                for path in paths {
                    assert!(text.contains(path.file_name().unwrap().to_str().unwrap()));
                    let bytes = std::fs::read(path).unwrap();
                    assert_eq!(
                        image::guess_format(&bytes).unwrap(),
                        if format == "webp" {
                            image::ImageFormat::WebP
                        } else {
                            image::ImageFormat::Png
                        }
                    );
                }
            }
            if tool == "compare_files" {
                let path = std::path::Path::new(
                    reply.structured_content.as_ref().unwrap()["reportJson"]
                        .as_str()
                        .unwrap(),
                );
                let report = std::fs::read_to_string(path).unwrap();
                assert!(report.contains(&format!(".{format}\"")));
                let paths: Vec<_> = std::fs::read_dir(path.parent().unwrap().join("render"))
                    .unwrap()
                    .collect();
                assert!(!paths.is_empty());
                for item in paths {
                    let path = item.unwrap().path();
                    assert_eq!(path.extension().unwrap(), format);
                    assert!(
                        image::load_from_memory(&std::fs::read(path).unwrap())
                            .unwrap()
                            .width()
                            > 0
                    );
                }
            }
        }
    }
    let comparison = call(
        &client,
        "compare_files",
        json!({"oldFile":"structure","newFile":"changed","includeRender":true}),
    )
    .await;
    assert_ne!(comparison.is_error, Some(true));
    let report = std::fs::read_to_string(
        comparison.structured_content.as_ref().unwrap()["reportJson"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert!(report.contains(".webp\""));
    let default = call(
        &client,
        "get_screenshot",
        json!({"fileKey":"structure","nodeId":node_id}),
    )
    .await;
    assert_eq!(
        serde_json::to_value(default).unwrap()["content"][0]["mimeType"],
        "image/png"
    );
    client.cancel().await.unwrap();
    service.cancel().await.unwrap();
}

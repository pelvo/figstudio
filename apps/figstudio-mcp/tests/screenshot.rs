use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp::tools::rendering::Rendering;
use figstudio_mcp::{FigstudioServer, tools::screenshot::screenshot};
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
};
use serde_json::{Value, json};
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig")
}
fn json_of(result: &CallToolResult) -> Value {
    serde_json::to_value(result).unwrap()
}
fn text(result: &CallToolResult) -> String {
    let content = json_of(result)["content"].clone();
    content
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "text")
        .unwrap()["text"]
        .as_str()
        .unwrap()
        .to_owned()
}
fn is_error(result: &CallToolResult) -> bool {
    result.is_error == Some(true)
}
/// Decode the image item into (width, height).
fn png_size(result: &CallToolResult) -> (u32, u32) {
    assert!(!is_error(result), "{}", text(result));
    let content = json_of(result)["content"].clone();
    let image = content
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "image")
        .unwrap();
    assert_eq!(image["mimeType"], "image/png");
    let bytes = STANDARD.decode(image["data"].as_str().unwrap()).unwrap();
    let png = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).unwrap();
    (png.width(), png.height())
}
fn doc() -> Document {
    Document::open(fixture()).unwrap()
}
fn shot(doc: &Document, id: &str, scale: Option<f64>) -> CallToolResult {
    screenshot(doc, id, scale, &Rendering::default())
}
fn frame(id: Guid, width: f64, height: f64, visible: bool) -> Node {
    child(id, Guid(0, 1), "FRAME", width, height, visible)
}
fn child(id: Guid, parent: Guid, kind: &str, width: f64, height: f64, visible: bool) -> Node {
    let size = Val::Obj(Obj::from([
        ("x", Val::Float(width)),
        ("y", Val::Float(height)),
    ]));
    let mut node = Node::from_data(Val::Obj(Obj::from([
        ("guid", id.to_val()),
        (
            "parentIndex",
            Val::Obj(Obj::from([("guid", parent.to_val())])),
        ),
        ("type", Val::Str(kind.into())),
        ("name", Val::Str("synthetic".into())),
        ("visible", Val::Bool(visible)),
    ])))
    .unwrap();
    node.data.set("size", size);
    node
}
fn with_frames(frames: Vec<Node>) -> Document {
    let mut doc = doc();
    let mut nodes = doc.nodes.clone();
    nodes.extend(frames);
    doc.set_nodes(nodes).unwrap();
    doc
}

/// The scale printed in the reply's first line.
fn reported_scale(reply: &str) -> f64 {
    reply
        .lines()
        .next()
        .unwrap()
        .rsplit("scale ")
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn png_is_cropped_to_the_node_box_and_follows_scale() {
    let doc = doc();
    // Node 1:34 is 381x191 design units: exactly that many pixels, no margin.
    assert_eq!(png_size(&shot(&doc, "1:34", None)), (381, 191));
    assert_eq!(png_size(&shot(&doc, "1-34", Some(0.5))), (191, 96));
    let reply = text(&shot(&doc, "1:34", Some(0.5)));
    assert!(reply.starts_with("191x96 px, scale 0.5"), "{reply}");
}

#[test]
fn a_text_box_is_not_padded_by_its_overhang() {
    let doc = with_frames(vec![child(
        Guid(9, 1),
        Guid(0, 1),
        "TEXT",
        100.0,
        20.0,
        true,
    )]);
    assert_eq!(png_size(&shot(&doc, "9:1", None)), (100, 20));
}

#[test]
fn page_and_url_ids_render_and_approximations_are_listed() {
    let doc = doc();
    let page = shot(&doc, "0:1", Some(0.1));
    assert!(png_size(&page).0 > 1);
    let url = "https://www.figma.com/design/key/name?node-id=1-2";
    let result = shot(&doc, url, Some(0.25));
    assert!(png_size(&result).0 > 100);
    // Node 1:2 is drawn with approximated blend modes today.
    let reply = text(&result);
    let line = reply
        .lines()
        .find_map(|l| l.strip_prefix("approximations: "))
        .unwrap();
    assert!(
        line.contains("blend modes approximated") && !line.contains("nodes"),
        "{reply}"
    );
}

#[test]
fn invalid_requests_are_tool_errors() {
    let doc = doc();
    for scale in [0.0, 0.005, 4.5, -1.0, f64::NAN, f64::INFINITY] {
        let result = shot(&doc, "1:34", Some(scale));
        assert!(is_error(&result), "scale {scale}");
        assert!(text(&result).contains("scale"));
    }
    for id in ["99:99", "bad", "I1:38;9:9", "0:0"] {
        assert!(is_error(&shot(&doc, id, None)), "{id}");
    }
}

#[test]
fn zero_size_and_hidden_nodes_are_errors() {
    let doc = with_frames(vec![
        frame(Guid(9, 1), 0.0, 0.0, true),
        frame(Guid(9, 2), 50.0, 50.0, false),
    ]);
    for id in ["9:1", "9:2"] {
        let result = shot(&doc, id, None);
        assert!(is_error(&result), "{id}");
        assert!(
            !json_of(&result)["content"]
                .to_string()
                .contains("\"image\"")
        );
    }
}

#[test]
fn explicit_scale_keeps_the_4096_cap_and_the_scale_is_honest() {
    let doc = with_frames(vec![frame(Guid(9, 1), 20_000.0, 100.0, true)]);
    let (w, h) = png_size(&shot(&doc, "9:1", Some(4.0)));
    assert!(w <= 4096 && w > 4000 && h < 100, "{w}x{h}");
    // Past 409 600 units the cap needs a scale below the renderer's old 0.01 floor.
    for width in [409_600.0, 500_000.0, 2_000_000.0] {
        let doc = with_frames(vec![frame(Guid(9, 1), width, 100.0, true)]);
        let result = shot(&doc, "9:1", Some(1.0));
        let (w, h) = png_size(&result);
        assert!(w <= 4096 && w > 4000 && h >= 1, "{width}: {w}x{h}");
        let scale = reported_scale(&text(&result));
        assert!(
            (scale * width - f64::from(w)).abs() <= 0.001 * f64::from(w),
            "{width}: scale {scale} gives {w} px"
        );
    }
}

#[test]
fn omitted_scale_caps_the_longest_side_at_2048() {
    // A 3000-unit frame at the default scale 1 would be 3000 px; the default cap is 2048.
    let doc = with_frames(vec![frame(Guid(9, 1), 3_000.0, 100.0, true)]);
    let result = shot(&doc, "9:1", None);
    let (w, h) = png_size(&result);
    assert!(w <= 2048 && w > 2000 && h < 100, "{w}x{h}");
    let scale = reported_scale(&text(&result));
    assert!(
        (scale * 3_000.0 - f64::from(w)).abs() <= 0.001 * f64::from(w),
        "scale {scale} gives {w} px"
    );
    // The same frame with an explicit scale 1 keeps all 3000 px (under the 4096 hard cap).
    let (w, _) = png_size(&shot(&doc, "9:1", Some(1.0)));
    assert_eq!(w, 3_000);
    // A default-scale request far past the cap stays at 2048 px in both dimensions.
    let big = with_frames(vec![frame(Guid(9, 1), 20_000.0, 100.0, true)]);
    let (w, _) = png_size(&shot(&big, "9:1", None));
    assert!(w <= 2048 && w > 2000, "{w}");
}

#[test]
fn omitted_subpixel_layers_are_reported() {
    let doc = with_frames(vec![
        frame(Guid(9, 1), 100.0, 100.0, true),
        child(Guid(9, 2), Guid(9, 1), "FRAME", 0.1, 0.1, true),
    ]);
    let reply = text(&shot(&doc, "9:1", None));
    assert!(
        reply.contains("approximations: 1 off screen or subpixel layers omitted"),
        "{reply}"
    );
    let clean = with_frames(vec![frame(Guid(9, 1), 100.0, 100.0, true)]);
    assert!(!text(&shot(&clean, "9:1", None)).contains("approximations"));
}

#[tokio::test]
async fn screenshot_tool_over_the_protocol() {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let (client_io, server_io) = tokio::io::duplex(1 << 20);
        let paths = vec![fixture()];
        let task = tokio::spawn(async move {
            FigstudioServer::new(paths, true)
                .serve(server_io)
                .await
                .unwrap()
                .waiting()
                .await
        });
        let client = ().serve(client_io).await.unwrap();
        let tools = client.list_all_tools().await.unwrap();
        let tool = tools
            .iter()
            .find(|tool| tool.name == "get_screenshot")
            .unwrap();
        assert!(tool.input_schema["properties"]["nodeId"].is_object());
        assert!(tool.input_schema["properties"]["scale"].is_object());
        let call = |args: Value| {
            client.call_tool(
                CallToolRequestParams::new("get_screenshot")
                    .with_arguments(args.as_object().unwrap().clone()),
            )
        };
        let ok = call(json!({"nodeId": "1:34", "scale": 0.5})).await.unwrap();
        let (w, _) = png_size(&ok);
        assert!(text(&ok).starts_with(&format!("{w}x")));
        for args in [
            json!({"nodeId": "99:99"}),
            json!({"nodeId": "1:34", "scale": 9}),
            json!({"nodeId": "1:34", "fileKey": "nope"}),
        ] {
            assert_eq!(call(args).await.unwrap().is_error, Some(true));
        }
        client.cancel().await.unwrap();
        task.await.unwrap().unwrap();
    })
    .await
    .expect("screenshot protocol timed out");
}

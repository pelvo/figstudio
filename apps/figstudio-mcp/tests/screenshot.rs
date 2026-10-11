use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_fig::{Document, Guid, Node, Obj, Val};
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
    screenshot(doc, id, scale, false)
}
fn frame(id: Guid, width: f64, height: f64, visible: bool) -> Node {
    let size = Val::Obj(Obj::from([
        ("x", Val::Float(width)),
        ("y", Val::Float(height)),
    ]));
    let mut node = Node::from_data(Val::Obj(Obj::from([
        ("guid", id.to_val()),
        (
            "parentIndex",
            Val::Obj(Obj::from([("guid", Guid(0, 1).to_val())])),
        ),
        ("type", Val::Str("FRAME".into())),
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

#[test]
fn png_size_covers_the_node_and_follows_scale() {
    let doc = doc();
    // Node 1:34 is 381x191 design units; the render adds a margin for strokes and effects.
    let (w1, h1) = png_size(&shot(&doc, "1:34", None));
    assert!(
        (381..381 + 60).contains(&w1) && (191..191 + 60).contains(&h1),
        "{w1}x{h1}"
    );
    let (w2, h2) = png_size(&shot(&doc, "1-34", Some(0.5)));
    assert!(
        w2.abs_diff(w1 / 2) <= 2 && h2.abs_diff(h1 / 2) <= 2,
        "{w2}x{h2}"
    );
    let reply = text(&shot(&doc, "1:34", Some(0.5)));
    assert!(
        reply.starts_with(&format!("{w2}x{h2} px, scale 0.5")),
        "{reply}"
    );
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
    for id in ["99:99", "bad", "I1:38;1:42", "0:0"] {
        assert!(is_error(&shot(&doc, id, None)), "{id}");
    }
    assert!(text(&shot(&doc, "I1:38;1:42", None)).contains("not yet supported"));
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
fn longest_side_is_capped_at_4096() {
    let doc = with_frames(vec![frame(Guid(9, 1), 20_000.0, 100.0, true)]);
    let (w, h) = png_size(&shot(&doc, "9:1", Some(4.0)));
    assert!(w <= 4096 && w > 4000 && h < 100, "{w}x{h}");
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

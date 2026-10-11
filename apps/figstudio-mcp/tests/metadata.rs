use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp::{FigstudioServer, tools::metadata::metadata};
use quick_xml::{Reader, events::Event};
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
};
use serde_json::json;
use std::{collections::HashMap, path::PathBuf};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig")
}
fn document() -> Document {
    Document::open(fixture()).unwrap()
}
fn text(result: &CallToolResult) -> String {
    serde_json::to_value(result).unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned()
}
fn elements(xml: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut reader = Reader::from_str(xml);
    let mut result = Vec::new();
    loop {
        match reader.read_event().unwrap() {
            Event::Start(tag) | Event::Empty(tag) => {
                let attrs = tag
                    .attributes()
                    .map(|attr| {
                        let attr = attr.unwrap();
                        (
                            String::from_utf8(attr.key.as_ref().to_vec()).unwrap(),
                            attr.decoded_and_normalized_value(
                                quick_xml::XmlVersion::Implicit1_0,
                                reader.decoder(),
                            )
                            .unwrap()
                            .into_owned(),
                        )
                    })
                    .collect();
                result.push((
                    String::from_utf8(tag.name().as_ref().to_vec()).unwrap(),
                    attrs,
                ));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    result
}
fn node(id: Guid, parent: Guid, kind: &str, name: &str) -> Node {
    Node::from_data(Val::Obj(Obj::from([
        ("guid", id.to_val()),
        (
            "parentIndex",
            Val::Obj(Obj::from([("guid", parent.to_val())])),
        ),
        ("type", Val::Str(kind.into())),
        ("name", Val::Str(name.into())),
    ])))
    .unwrap()
}

#[test]
fn public_fixture_page_list_subtree_and_relative_coordinates() {
    let doc = document();
    let pages = elements(&text(&metadata(&doc, None, None)));
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1].0, "canvas");
    assert_eq!(pages[1].1["id"], "0:1");
    assert_eq!(pages[1].1["name"], "Page 1");
    let subtree = elements(&text(&metadata(&doc, Some("0:1"), None)));
    for kind in [
        "canvas",
        "frame",
        "symbol",
        "text",
        "instance",
        "rounded-rectangle",
        "vector",
    ] {
        assert!(
            subtree.iter().any(|(name, _)| name == kind),
            "missing {kind}"
        );
    }
    let (_, attrs) = subtree
        .iter()
        .find(|(_, a)| a.get("id").is_some_and(|id| id == "1:42"))
        .unwrap();
    assert_eq!(attrs["x"], "53");
    assert_eq!(attrs["y"], "15");
}

#[test]
fn depth_zero_is_root_only_and_one_includes_children() {
    let doc = document();
    assert_eq!(
        elements(&text(&metadata(&doc, Some("0:1"), Some(0)))).len(),
        2
    );
    let xml = text(&metadata(&doc, Some("0:1"), Some(1)));
    let nodes = elements(&xml);
    assert_eq!(nodes.len(), 4);
    assert!(!xml.contains("id=\"1:42\""));
}

#[test]
fn synthetic_attributes_text_escaping_and_instance_expansion() {
    let mut doc = document();
    let root = Guid(9, 1);
    let mut frame = node(root, Guid(0, 1), "FRAME", "<&\"");
    frame.data.set(
        "transform",
        Val::Obj(Obj::from([
            ("m00", Val::Float(0.0)),
            ("m10", Val::Float(1.0)),
            ("m02", Val::Float(-0.0001)),
            ("m12", Val::Float(12.34567)),
        ])),
    );
    frame.data.set(
        "size",
        Val::Obj(Obj::from([
            ("x", Val::Float(80.0)),
            ("y", Val::Float(90.0)),
        ])),
    );
    frame.data.set("visible", Val::Bool(false));
    let mut label = node(Guid(9, 2), root, "TEXT", "label");
    label.data.set(
        "textData",
        Val::Obj(Obj::from([(
            "characters",
            Val::Str(format!("<&{}tail", "é".repeat(210))),
        )])),
    );
    let mut instance = node(Guid(9, 3), root, "INSTANCE", "instance");
    instance.data.set(
        "symbolData",
        Val::Obj(Obj::from([("symbolID", Guid(1, 37).to_val())])),
    );
    // Stored children of an instance are not its content; the component's are.
    let child = node(Guid(9, 4), instance.id, "FRAME", "must not expand");
    let mut nodes = doc.nodes.clone();
    let mut counter_clockwise = node(Guid(9, 5), root, "FRAME", "counter-clockwise");
    counter_clockwise.data.set(
        "transform",
        Val::Obj(Obj::from([
            ("m00", Val::Float(0.0)),
            ("m10", Val::Float(-1.0)),
        ])),
    );
    nodes.extend([frame, label, instance, child, counter_clockwise]);
    doc.set_nodes(nodes).unwrap();
    let xml = text(&metadata(&doc, Some("9:1"), None));
    let tags = elements(&xml);
    let attrs = &tags[1].1;
    assert_eq!(attrs["name"], "<&\"");
    assert_eq!(attrs["x"], "0");
    assert_eq!(attrs["y"], "12.346");
    assert_eq!(attrs["width"], "80");
    assert_eq!(attrs["height"], "90");
    assert_eq!(attrs["hidden"], "true");
    assert_eq!(attrs["rotation"], "-90");
    let opposite = tags
        .iter()
        .find(|(_, attrs)| attrs.get("id").is_some_and(|id| id == "9:5"))
        .unwrap();
    assert_eq!(opposite.1["rotation"], "90");
    assert!(xml.contains(&format!("&lt;&amp;{}…", "é".repeat(198))));
    assert!(!xml.contains("tail"));
    assert!(!xml.contains("id=\"9:4\""));
    assert!(xml.contains("id=\"I9:3;1:42\""));
    assert_eq!(
        tags.iter().find(|(kind, _)| kind == "instance").unwrap().1["componentId"],
        "1:37"
    );
}

#[test]
fn output_caps_at_five_thousand_and_marks_truncation() {
    let mut doc = document();
    let root = Guid(9, 1);
    let mut nodes = doc.nodes.clone();
    nodes.push(node(root, Guid(0, 1), "FRAME", "root"));
    for index in 2..=5100 {
        nodes.push(node(Guid(9, index), root, "FRAME", "child"));
    }
    doc.set_nodes(nodes).unwrap();
    let xml = text(&metadata(&doc, Some("9:1"), None));
    let tags = elements(&xml);
    assert_eq!(tags.len(), 5001);
    assert_eq!(tags[0].1["truncated"], "true");
    let mut nodes = doc.nodes.clone();
    nodes.retain(|node| node.id.0 != 9 || node.id.1 <= 5000);
    doc.set_nodes(nodes).unwrap();
    let tags = elements(&text(&metadata(&doc, Some("9:1"), None)));
    assert_eq!(tags.len(), 5001);
    assert!(!tags[0].1.contains_key("truncated"));
}

#[test]
fn deeply_nested_subtrees_are_bounded_without_recursion() {
    let mut doc = document();
    let mut nodes = doc.nodes.clone();
    for index in 1..=5100 {
        let parent = if index == 1 {
            Guid(0, 1)
        } else {
            Guid(9, index - 1)
        };
        nodes.push(node(Guid(9, index), parent, "FRAME", "nested"));
    }
    doc.set_nodes(nodes).unwrap();
    let tags = elements(&text(&metadata(&doc, Some("9:1"), None)));
    assert_eq!(tags.len(), 5001);
    assert_eq!(tags[0].1["truncated"], "true");
}

#[test]
fn invisible_and_internal_pages_are_excluded() {
    let mut doc = document();
    let mut hidden = node(Guid(9, 1), doc.root.unwrap(), "CANVAS", "hidden page");
    hidden.data.set("visible", Val::Bool(false));
    let mut internal = node(Guid(9, 2), doc.root.unwrap(), "CANVAS", "internal page");
    internal.data.set("internalOnly", Val::Bool(true));
    let mut nodes = doc.nodes.clone();
    nodes.extend([hidden, internal]);
    doc.set_nodes(nodes).unwrap();
    let xml = text(&metadata(&doc, None, None));
    assert!(!xml.contains("hidden page"));
    assert!(!xml.contains("internal page"));
    assert!(xml.contains("Page 1"));
}

#[tokio::test]
async fn metadata_tool_handles_ids_pages_and_missing_files() {
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        check_metadata_protocol(),
    )
    .await
    .expect("metadata protocol timed out");
}

async fn check_metadata_protocol() {
    let paths = vec![fixture(), fixture().with_file_name("missing.fig")];
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
    let tools = client.list_all_tools().await.unwrap();
    let tool = tools
        .iter()
        .find(|tool| tool.name == "get_metadata")
        .unwrap();
    assert!(tool.input_schema["properties"]["nodeId"].is_object());
    for args in [
        json!({"fileKey":"structure"}),
        json!({"fileKey":"structure","nodeId":"0-1"}),
        json!({"fileKey":"structure","nodeId":"https://www.figma.com/design/example?node-id=0-1"}),
    ] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("get_metadata")
                    .with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true));
        assert!(text(&result).contains("id=\"0:1\""));
    }
    for input in ["99:99", "bad", "I1:38;9:9"] {
        let args = json!({"fileKey":"structure", "nodeId": input});
        let result = client
            .call_tool(
                CallToolRequestParams::new("get_metadata")
                    .with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(text(&result).contains("Available pages"));
        assert!(text(&result).contains("id=\"0:1\""));
    }
    for file in [Some("missing"), Some("unregistered"), None] {
        let args = file.map_or(json!({}), |file| json!({"fileKey":file}));
        let request = CallToolRequestParams::new("get_metadata")
            .with_arguments(args.as_object().unwrap().clone());
        assert_eq!(
            client.call_tool(request).await.unwrap().is_error,
            Some(true)
        );
    }
    client.cancel().await.unwrap();
    task.await.unwrap().unwrap();
}

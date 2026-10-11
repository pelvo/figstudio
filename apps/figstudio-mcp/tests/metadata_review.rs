use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp::tools::metadata::metadata;
use quick_xml::{Reader, events::Event};
use std::{collections::HashMap, path::PathBuf};

fn document() -> Document {
    Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
    )
    .unwrap()
}
fn xml(doc: &Document, id: Option<&str>, depth: Option<u32>) -> String {
    serde_json::to_value(metadata(doc, id, depth)).unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned()
}
fn node(id: u32, parent: Guid, kind: &str, name: &str) -> Node {
    Node::from_data(Val::Obj(Obj::from([
        ("guid", Guid(9, id).to_val()),
        (
            "parentIndex",
            Val::Obj(Obj::from([("guid", parent.to_val())])),
        ),
        ("type", Val::Str(kind.into())),
        ("name", Val::Str(name.into())),
    ])))
    .unwrap()
}
fn attrs(output: &str, id: &str) -> HashMap<String, String> {
    let mut reader = Reader::from_str(output);
    loop {
        match reader.read_event().unwrap() {
            Event::Start(tag) | Event::Empty(tag) => {
                let attrs: HashMap<_, _> = tag
                    .attributes()
                    .map(|a| {
                        let a = a.unwrap();
                        (
                            String::from_utf8(a.key.as_ref().to_vec()).unwrap(),
                            a.decoded_and_normalized_value(
                                quick_xml::XmlVersion::Implicit1_0,
                                reader.decoder(),
                            )
                            .unwrap()
                            .into_owned(),
                        )
                    })
                    .collect();
                if attrs.get("id").is_some_and(|value| value == id) {
                    return attrs;
                }
            }
            Event::Eof => panic!("missing node {id}"),
            _ => {}
        }
    }
}

#[test]
fn page_and_document_geometry_is_absent_and_unsized_nodes_have_no_dimensions() {
    let mut doc = document();
    let root = doc.root.unwrap().to_string();
    let mut nodes = doc.nodes.clone();
    nodes.push(node(1, Guid(0, 1), "FRAME", "unsized"));
    doc.set_nodes(nodes).unwrap();
    for id in [root.as_str(), "0:1"] {
        let attributes = attrs(&xml(&doc, Some(id), Some(0)), id);
        for key in ["x", "y", "width", "height"] {
            assert!(!attributes.contains_key(key), "{id} has {key}");
        }
    }
    let attributes = attrs(&xml(&doc, Some("9:1"), None), "9:1");
    assert_eq!(attributes["x"], "0");
    for key in ["width", "height"] {
        assert!(!attributes.contains_key(key));
    }
}

#[test]
fn xml_controls_are_removed_from_page_names_node_names_and_text() {
    let mut doc = document();
    let controls: String = (0..=31)
        .filter(|n| ![9, 10, 13].contains(n))
        .map(|n| char::from_u32(n).unwrap())
        .collect();
    let dirty = format!("before{controls}after");
    let mut nodes = doc.nodes.clone();
    nodes.iter_mut().find(|n| n.id == Guid(0, 1)).unwrap().name = dirty.clone();
    let mut label = node(1, Guid(0, 1), "TEXT", &dirty);
    label.data.set(
        "textData",
        Val::Obj(Obj::from([(
            "characters",
            Val::Str(format!("{dirty}\t\n\r")),
        )])),
    );
    nodes.push(label);
    doc.set_nodes(nodes).unwrap();
    for output in [xml(&doc, None, None), xml(&doc, Some("9:1"), None)] {
        assert!(!output.chars().any(|c| controls.contains(c)));
        assert!(output.contains("beforeafter"));
    }
    assert_eq!(
        attrs(&xml(&doc, Some("9:1"), None), "9:1")["name"],
        "beforeafter"
    );
    assert!(xml(&doc, Some("9:1"), None).contains("\t\n\r"));
}

#[test]
fn text_marks_only_actual_truncation_and_empty_leaves_self_close() {
    let mut doc = document();
    let mut nodes = doc.nodes.clone();
    for (id, length) in [(1, 200), (2, 201), (3, 0)] {
        let mut label = node(id, Guid(0, 1), "TEXT", "label");
        label.data.set(
            "textData",
            Val::Obj(Obj::from([("characters", Val::Str("é".repeat(length)))])),
        );
        nodes.push(label);
    }
    nodes.push(node(4, Guid(0, 1), "FRAME", "leaf"));
    doc.set_nodes(nodes).unwrap();
    assert!(!xml(&doc, Some("9:1"), None).contains('…'));
    assert!(xml(&doc, Some("9:2"), None).contains(&format!("{}…", "é".repeat(200))));
    for id in ["9:3", "9:4"] {
        let output = xml(&doc, Some(id), None);
        assert!(output.contains("/>"));
        assert!(!output.contains("approximations:"));
    }
    assert!(!xml(&doc, Some("0:1"), Some(0)).contains("approximations:"));
    assert!(xml(&doc, Some("1:38"), None).contains("approximations: 1"));
}

#[test]
fn document_subtrees_exclude_internal_canvases_and_their_descendants() {
    let mut doc = document();
    let root = doc.root.unwrap();
    let mut internal = node(1, root, "CANVAS", "internal storage");
    internal.data.set("internalOnly", Val::Bool(true));
    let mut nodes = doc.nodes.clone();
    nodes.extend([internal, node(2, Guid(9, 1), "FRAME", "internal child")]);
    doc.set_nodes(nodes).unwrap();
    let output = xml(&doc, Some(&root.to_string()), None);
    assert!(!output.contains("internal storage"));
    assert!(!output.contains("internal child"));
    assert!(output.contains("Page 1"));
}

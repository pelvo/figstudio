//! Instance expansion and instance paths in get_metadata, get_screenshot and get_variable_defs.
use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp::tools::{metadata::metadata, screenshot::screenshot, variables::variable_defs};
use rmcp::model::CallToolResult;
use std::path::PathBuf;

fn document() -> Document {
    Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
    )
    .unwrap()
}
fn json(result: &CallToolResult) -> serde_json::Value {
    serde_json::to_value(result).unwrap()
}
fn text(result: &CallToolResult) -> String {
    json(result)["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_owned()
}
fn xml(doc: &Document, id: &str, depth: Option<u32>) -> String {
    let result = metadata(doc, Some(id), depth);
    assert_ne!(result.is_error, Some(true), "{id}: {}", text(&result));
    text(&result)
}
fn obj(fields: &[(&str, Val)]) -> Val {
    let mut out = Obj::new();
    for (key, value) in fields {
        out.insert(key, value.clone());
    }
    Val::Obj(out)
}
fn node(id: u32, parent: Guid, kind: &str, name: &str) -> Node {
    Node::from_data(obj(&[
        ("guid", Guid(9, id).to_val()),
        ("parentIndex", obj(&[("guid", parent.to_val())])),
        ("type", Val::Str(kind.into())),
        ("name", Val::Str(name.into())),
        (
            "size",
            obj(&[("x", Val::Float(10.0)), ("y", Val::Float(10.0))]),
        ),
    ]))
    .unwrap()
}

#[test]
fn fixture_instances_expand_with_overridden_text_and_component_ids() {
    let doc = document();
    let page = xml(&doc, "1:4", None);
    for want in [
        r#"<instance id="1:38" name="Component 2""#,
        r#"componentId="1:37""#,
        r#"<text id="I1:38;1:42" name="ABC""#,
        ">XYZ</text>",
        r#"<rounded-rectangle id="I1:45;1:35""#,
    ] {
        assert!(page.contains(want), "missing {want} in {page}");
    }
    assert!(!page.contains("approximations:"));
    let child = xml(&doc, "I1:38;1:42", None);
    assert!(
        child.starts_with(r#"<metadata><text id="I1:38;1:42""#),
        "{child}"
    );
    assert!(child.contains(">XYZ</text>"));
    assert_eq!(xml(&doc, "1:38", Some(0)).matches("id=").count(), 1);
    assert_eq!(xml(&doc, "1:38", Some(1)).matches("id=").count(), 3);
    let missing = metadata(&doc, Some("I1:38;9:9"), None);
    assert_eq!(missing.is_error, Some(true));
}

/// Component 9:2 holds a hidden-by-property rectangle and a text; instance 9:10 sets the
/// property and overrides the text and the rectangle's name.
fn override_doc() -> Document {
    let mut doc = document();
    let page = Guid(0, 1);
    let mut symbol = node(2, page, "SYMBOL", "component");
    symbol.data.set(
        "componentPropDefs",
        Val::Arr(vec![obj(&[("id", Guid(9, 90).to_val())])]),
    );
    let mut rect = node(3, symbol.id, "RECTANGLE", "box");
    rect.data.set(
        "componentPropRefs",
        Val::Arr(vec![obj(&[
            ("defID", Guid(9, 90).to_val()),
            ("componentPropNodeField", Val::Str("VISIBLE".into())),
        ])]),
    );
    let mut label = node(4, symbol.id, "TEXT", "label");
    label.data.set(
        "textData",
        obj(&[("characters", Val::Str("before".into()))]),
    );
    let mut top = node(10, page, "INSTANCE", "instance");
    let guids = |n: u32| obj(&[("guids", Val::Arr(vec![Guid(9, n).to_val()]))]);
    let overrides = vec![
        obj(&[
            ("guidPath", guids(4)),
            ("textData", obj(&[("characters", Val::Str("after".into()))])),
        ]),
        obj(&[("guidPath", guids(3)), ("name", Val::Str("renamed".into()))]),
    ];
    top.data.set(
        "symbolData",
        obj(&[
            ("symbolID", symbol.id.to_val()),
            ("symbolOverrides", Val::Arr(overrides)),
        ]),
    );
    top.data.set(
        "componentPropAssignments",
        Val::Arr(vec![obj(&[
            ("defID", Guid(9, 90).to_val()),
            ("value", obj(&[("boolValue", Val::Bool(false))])),
        ])]),
    );
    let mut nodes = doc.nodes.clone();
    nodes.extend([symbol, rect, label, top]);
    doc.set_nodes(nodes).unwrap();
    doc
}

#[test]
fn built_overrides_show_in_metadata() {
    let out = xml(&override_doc(), "9:10", None);
    assert!(
        out.contains(r#"<rectangle id="I9:10;9:3" name="renamed""#),
        "{out}"
    );
    assert!(out.contains(r#"hidden="true""#), "{out}");
    assert!(out.contains(">after</text>"), "{out}");
}

#[test]
fn a_missing_component_is_counted_not_expanded() {
    let mut doc = document();
    let mut broken = node(10, Guid(0, 1), "INSTANCE", "broken");
    broken
        .data
        .set("symbolData", obj(&[("symbolID", Guid(9, 99).to_val())]));
    let mut nodes = doc.nodes.clone();
    nodes.push(broken);
    doc.set_nodes(nodes).unwrap();
    let out = xml(&doc, "9:10", None);
    assert!(out.contains("<instance id=\"9:10\""), "{out}");
    assert!(
        out.contains("approximations: 1 instances not expanded"),
        "{out}"
    );
}

#[test]
fn screenshots_render_an_instance_child_cropped_to_its_box() {
    let doc = document();
    let shot = screenshot(&doc, "I1:38;1:35", Some(1.0), false);
    assert_ne!(shot.is_error, Some(true), "{}", text(&shot));
    assert_eq!(
        json(&shot)["content"][1]["text"]
            .as_str()
            .unwrap()
            .lines()
            .next(),
        Some("45x45 px, scale 1")
    );
    // The child has no override, so it looks exactly like the component's own child.
    let original = screenshot(&doc, "1:35", Some(1.0), false);
    assert_eq!(
        json(&shot)["content"][0]["data"],
        json(&original)["content"][0]["data"]
    );
    // An offset child: the crop must follow its transform inside the instance.
    let offset = screenshot(&doc, "I1:45;1:42", Some(1.0), false);
    let label = screenshot(&doc, "1:42", Some(1.0), false);
    assert_eq!(
        json(&offset)["content"][0]["data"],
        json(&label)["content"][0]["data"]
    );
    let missing = screenshot(&doc, "I1:38;9:9", None, false);
    assert_eq!(missing.is_error, Some(true));
}

#[test]
fn variable_defs_walk_resolved_instance_content() {
    let doc = document();
    for id in ["1:38", "I1:38;1:42"] {
        let result = variable_defs(&doc, id);
        assert_ne!(result.is_error, Some(true), "{id}: {}", text(&result));
    }
    assert_eq!(variable_defs(&doc, "I1:38;9:9").is_error, Some(true));
}

//! Review regressions: instance paths by tree position, scaled crops, effective references.
use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp::tools::rendering::Rendering;
use figstudio_mcp::tools::{metadata::metadata, screenshot::screenshot, variables::variable_defs};
use rmcp::model::CallToolResult;
use std::path::PathBuf;

fn obj(fields: &[(&str, Val)]) -> Val {
    let mut out = Obj::new();
    for (key, value) in fields {
        out.insert(key, value.clone());
    }
    Val::Obj(out)
}
fn g(n: u32) -> Val {
    Guid(9, n).to_val()
}
fn node(n: u32, parent: Option<u32>, kind: &str, fields: &[(&str, Val)]) -> Node {
    let parent = parent.map_or(Guid(0, 1), |p| Guid(9, p));
    let mut data = obj(&[
        ("guid", g(n)),
        ("parentIndex", obj(&[("guid", parent.to_val())])),
        ("type", Val::Str(kind.into())),
        ("name", Val::Str(format!("n{n}"))),
        (
            "size",
            obj(&[("x", Val::Float(40.0)), ("y", Val::Float(40.0))]),
        ),
    ]);
    for (key, value) in fields {
        data.set(key, value.clone());
    }
    Node::from_data(data).unwrap()
}
fn instance(n: u32, symbol: u32, fields: &[(&str, Val)]) -> Node {
    let mut node = node(n, None, "INSTANCE", fields);
    if node.data.get("symbolData").is_none() {
        node.data.set("symbolData", obj(&[("symbolID", g(symbol))]));
    }
    node
}
fn translate(x: f64) -> Val {
    obj(&[
        ("m00", Val::Float(1.0)),
        ("m11", Val::Float(1.0)),
        ("m02", Val::Float(x)),
    ])
}
fn red() -> Val {
    let color = [("r", 1.0), ("g", 0.0), ("b", 0.0), ("a", 1.0)].map(|(k, v)| (k, Val::Float(v)));
    Val::Arr(vec![obj(&[
        ("type", Val::Str("SOLID".into())),
        ("color", obj(&color)),
    ])])
}
fn with(nodes: Vec<Node>) -> Document {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let mut doc = Document::open(path).unwrap();
    let mut all = doc.nodes.clone();
    all.extend(nodes);
    doc.set_nodes(all).unwrap();
    doc
}
fn text(result: &CallToolResult) -> String {
    let value = serde_json::to_value(result).unwrap();
    value["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Component 9:2 → frame 9:3 → rectangle 9:4 (a frame between the instance and the target).
fn framed() -> Document {
    with(vec![
        node(2, None, "SYMBOL", &[]),
        node(3, Some(2), "FRAME", &[]),
        node(4, Some(3), "RECTANGLE", &[]),
        node(5, Some(4), "RECTANGLE", &[]),
        instance(10, 2, &[]),
    ])
}

#[test]
fn instance_paths_count_tree_levels_from_the_target() {
    let doc = framed();
    let root = text(&metadata(&doc, Some("I9:10;9:4"), Some(0)));
    assert!(root.contains(r#"id="I9:10;9:4""#), "{root}");
    assert!(
        !root.contains(r#"I9:10;9:5"#),
        "depth 0 is the target only: {root}"
    );
    let one = text(&metadata(&doc, Some("I9:10;9:4"), Some(1)));
    assert!(one.contains(r#"id="I9:10;9:5""#), "{one}");
    let frame = text(&metadata(&doc, Some("I9:10;9:3"), Some(1)));
    assert!(frame.contains(r#"id="I9:10;9:4""#), "{frame}");
}

#[test]
fn a_swapped_nested_instance_reports_its_new_component() {
    let swap = obj(&[
        ("guidPath", obj(&[("guids", Val::Arr(vec![g(3)]))])),
        ("overriddenSymbolID", g(6)),
    ]);
    let doc = with(vec![
        node(2, None, "SYMBOL", &[]),
        node(
            3,
            Some(2),
            "INSTANCE",
            &[("symbolData", obj(&[("symbolID", g(4))]))],
        ),
        node(4, None, "SYMBOL", &[]),
        node(6, None, "SYMBOL", &[]),
        instance(
            10,
            2,
            &[(
                "symbolData",
                obj(&[
                    ("symbolID", g(2)),
                    ("symbolOverrides", Val::Arr(vec![swap])),
                ]),
            )],
        ),
    ]);
    let out = text(&metadata(&doc, Some("9:10"), None));
    assert!(
        out.contains(
            r#"id="I9:10;9:3" name="n3" x="0" y="0" width="40" height="40" componentId="9:6""#
        ),
        "{out}"
    );
}

#[test]
fn a_scaled_instance_child_is_cropped_where_it_renders_and_alone() {
    let doc = with(vec![
        node(
            2,
            None,
            "SYMBOL",
            &[(
                "size",
                obj(&[("x", Val::Float(120.0)), ("y", Val::Float(40.0))]),
            )],
        ),
        node(
            3,
            Some(2),
            "RECTANGLE",
            &[("transform", translate(80.0)), ("fillPaints", red())],
        ),
        // A sibling overlapping the child's box must not appear in the child's screenshot.
        node(
            4,
            Some(2),
            "RECTANGLE",
            &[("transform", translate(60.0)), ("fillPaints", red())],
        ),
        instance(
            10,
            2,
            &[
                (
                    "size",
                    obj(&[("x", Val::Float(240.0)), ("y", Val::Float(80.0))]),
                ),
                (
                    "symbolData",
                    obj(&[("symbolID", g(2)), ("uniformScaleFactor", Val::Float(2.0))]),
                ),
            ],
        ),
    ]);
    let shot = screenshot(&doc, "I9:10;9:3", Some(1.0), &Rendering::default());
    assert!(text(&shot).starts_with("80x80 px"), "{}", text(&shot));
    let value = serde_json::to_value(&shot).unwrap();
    let data = value["content"][0]["data"].as_str().unwrap();
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap();
    let image = image::load_from_memory(&png).unwrap().to_rgba8();
    let painted = image.pixels().filter(|p| p.0 == [255, 0, 0, 255]).count();
    assert_eq!(
        painted,
        80 * 80,
        "the child fills the crop (it rendered at 160..240)"
    );
    // Hiding the child: the sibling must not show through the crop.
    let hidden = obj(&[
        ("guidPath", obj(&[("guids", Val::Arr(vec![g(3)]))])),
        ("visible", Val::Bool(false)),
    ]);
    let mut nodes = doc.nodes.clone();
    let top = nodes.iter_mut().find(|n| n.id == Guid(9, 10)).unwrap();
    top.data
        .get_mut("symbolData")
        .unwrap()
        .set("symbolOverrides", Val::Arr(vec![hidden]));
    let mut doc = doc;
    doc.set_nodes(nodes).unwrap();
    assert_eq!(
        screenshot(&doc, "I9:10;9:3", Some(1.0), &Rendering::default()).is_error,
        Some(true)
    );
}

#[test]
fn variable_defs_ignore_unused_override_and_property_storage() {
    let style = |n: u32| obj(&[("guid", g(n))]);
    let fill_style = |n: u32, name: &str| {
        node(
            n,
            None,
            "RECTANGLE",
            &[
                ("styleType", Val::Str("FILL".into())),
                ("name", Val::Str(name.into())),
                ("fillPaints", red()),
            ],
        )
    };
    let mut styles = vec![
        fill_style(30, "Used"),
        fill_style(31, "Superseded"),
        fill_style(32, "Orphan"),
    ];
    for s in &mut styles {
        s.parent = None;
    }
    // Instance 9:12 shows component 9:14, which holds instance 9:13 of component 9:2. 9:13's own
    // override styles 9:3 with "Superseded"; 9:12's outer override restyles it "Used" and wins.
    // An override keyed to a node that does not exist ("Orphan") styles nothing.
    let patch = |path: &[u32], n: u32| {
        let guids = Val::Arr(path.iter().map(|p| g(*p)).collect());
        obj(&[
            ("guidPath", obj(&[("guids", guids)])),
            ("styleIdForFill", style(n)),
        ])
    };
    let symbol_data = |symbol: u32, patches: Vec<Val>| {
        obj(&[
            ("symbolID", g(symbol)),
            ("symbolOverrides", Val::Arr(patches)),
        ])
    };
    let mut nodes = vec![
        node(2, None, "SYMBOL", &[]),
        node(3, Some(2), "RECTANGLE", &[]),
        node(14, None, "SYMBOL", &[]),
        node(
            13,
            Some(14),
            "INSTANCE",
            &[("symbolData", symbol_data(2, vec![patch(&[3], 31)]))],
        ),
        instance(
            12,
            14,
            &[(
                "symbolData",
                symbol_data(14, vec![patch(&[13, 3], 30), patch(&[99], 32)]),
            )],
        ),
    ];
    nodes.extend(styles);
    let doc = with(nodes);
    for id in ["9:12", "I9:12;9:13"] {
        let map = variable_defs(&doc, id).structured_content.unwrap();
        assert!(map.get("Used").is_some(), "{id}: {map}");
        assert!(
            map.get("Superseded").is_none(),
            "{id}: superseded override leaked: {map}"
        );
        assert!(
            map.get("Orphan").is_none(),
            "{id}: unused override leaked: {map}"
        );
    }
}

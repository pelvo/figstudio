//! Uniformly scaled instances (`symbolData.uniformScaleFactor`): get_metadata,
//! get_design_context and get_screenshot report the geometry the renderer draws.
use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp::tools::design::{Assets, Options, design_context};
use figstudio_mcp::tools::rendering::Rendering;
use figstudio_mcp::tools::{metadata::metadata, screenshot::screenshot};
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
fn wh(w: f64, h: f64) -> Val {
    obj(&[("x", Val::Float(w)), ("y", Val::Float(h))])
}
fn at(x: f64, y: f64) -> Val {
    obj(&[
        ("m00", Val::Float(1.0)),
        ("m11", Val::Float(1.0)),
        ("m02", Val::Float(x)),
        ("m12", Val::Float(y)),
    ])
}
fn red() -> Val {
    let color = [("r", 1.0), ("g", 0.0), ("b", 0.0), ("a", 1.0)].map(|(k, v)| (k, Val::Float(v)));
    Val::Arr(vec![obj(&[
        ("type", Val::Str("SOLID".into())),
        ("color", obj(&color)),
    ])])
}
fn node(n: u32, parent: Option<u32>, kind: &str, fields: &[(&str, Val)]) -> Node {
    let parent = parent.map_or(Guid(0, 1), |p| Guid(9, p));
    let mut data = obj(&[
        ("guid", g(n)),
        ("parentIndex", obj(&[("guid", parent.to_val())])),
        ("type", Val::Str(kind.into())),
        ("name", Val::Str(format!("n{n}"))),
    ]);
    for (key, value) in fields {
        data.set(key, value.clone());
    }
    Node::from_data(data).unwrap()
}
/// An instance of `symbol` with `size` at `y`, scaled by `scale` when it is set.
fn instance(n: u32, parent: Option<u32>, symbol: u32, size: Val, y: f64, scale: f64) -> Node {
    let mut symbol_data = obj(&[("symbolID", g(symbol))]);
    if scale != 1.0 {
        symbol_data.set("uniformScaleFactor", Val::Float(scale));
    }
    node(
        n,
        parent,
        "INSTANCE",
        &[
            ("size", size),
            ("transform", at(0.0, y)),
            ("symbolData", symbol_data),
        ],
    )
}

/// Component 9:2 (100x50) holds rectangle 9:3 (30x20 at 10,5, stroke 2, radius 4) and
/// instance 9:6 of component 9:4 scaled by 1.5 (30x30 at 50,10), whose rectangle 9:5 is 6x4
/// at 2,3. Instance 9:10 scales 9:2 by 2; 9:11 is resized without a uniform scale; 9:12 scales
/// by 2 but its own derived data states 9:3's final size.
fn document() -> Document {
    let rect = |n, parent, x, y, w, h| {
        node(
            n,
            Some(parent),
            "RECTANGLE",
            &[
                ("size", wh(w, h)),
                ("transform", at(x, y)),
                ("fillPaints", red()),
                ("strokePaints", red()),
                ("strokeWeight", Val::Float(2.0)),
                ("cornerRadius", Val::Float(4.0)),
            ],
        )
    };
    let mut scaled_derived = instance(12, None, 2, wh(200.0, 100.0), 600.0, 2.0);
    scaled_derived.data.set(
        "derivedSymbolData",
        Val::Arr(vec![obj(&[
            ("guidPath", obj(&[("guids", Val::Arr(vec![g(3)]))])),
            ("size", wh(61.0, 41.0)),
        ])]),
    );
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let mut doc = Document::open(path).unwrap();
    let mut nodes = doc.nodes.clone();
    nodes.extend([
        node(4, None, "SYMBOL", &[("size", wh(20.0, 20.0))]),
        rect(5, 4, 2.0, 3.0, 6.0, 4.0),
        node(2, None, "SYMBOL", &[("size", wh(100.0, 50.0))]),
        rect(3, 2, 10.0, 5.0, 30.0, 20.0),
        {
            let mut nested = instance(6, Some(2), 4, wh(30.0, 30.0), 10.0, 1.5);
            nested.data.set("transform", at(50.0, 10.0));
            nested
        },
        instance(10, None, 2, wh(200.0, 100.0), 200.0, 2.0),
        instance(11, None, 2, wh(150.0, 75.0), 400.0, 1.0),
        scaled_derived,
    ]);
    doc.set_nodes(nodes).unwrap();
    doc
}

fn texts(result: &CallToolResult) -> Vec<String> {
    assert_ne!(result.is_error, Some(true), "{result:?}");
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect()
}
fn geometry(x: f64, y: f64, w: f64, h: f64) -> String {
    format!(r#"x="{x}" y="{y}" width="{w}" height="{h}""#)
}
/// Pixel size line of a screenshot at `scale`.
fn shot(doc: &Document, id: &str, scale: f64) -> String {
    let result = screenshot(doc, id, Some(scale), &Rendering::default());
    texts(&result)[0].lines().next().unwrap().to_owned()
}

#[test]
fn metadata_reports_scaled_child_geometry_like_the_screenshot() {
    let doc = document();
    let xml = texts(&metadata(&doc, Some("9:10"), None)).remove(0);
    for (id, want) in [
        ("I9:10;9:3", geometry(20.0, 10.0, 60.0, 40.0)),
        ("I9:10;9:6", geometry(100.0, 20.0, 60.0, 60.0)),
        // Nested: the inner 1.5 and the outer 2 multiply.
        ("I9:10;9:6;9:5", geometry(6.0, 9.0, 18.0, 12.0)),
    ] {
        assert!(xml.contains(&format!(r#"id="{id}" name"#)), "{xml}");
        let element = xml
            .lines()
            .find(|l| l.contains(&format!("\"{id}\"")))
            .unwrap();
        assert!(element.contains(&want), "{id}: want {want}: {element}");
        // The same id looked up on its own agrees with the walk.
        let alone = texts(&metadata(&doc, Some(id), Some(0))).remove(0);
        assert!(alone.contains(&want), "{id} alone: {alone}");
    }
    for scale in [1.0, 0.5] {
        assert_eq!(
            shot(&doc, "I9:10;9:3", scale),
            format!("{}x{} px, scale {scale}", 60.0 * scale, 40.0 * scale)
        );
    }
    assert_eq!(shot(&doc, "I9:10;9:6;9:5", 1.0), "18x12 px, scale 1");
}

#[test]
fn unscaled_and_owner_stated_geometry_is_not_scaled_again() {
    let doc = document();
    // Resized by constraints, no uniform scale: the child keeps the component's geometry.
    let resized = texts(&metadata(&doc, Some("I9:11;9:3"), Some(0))).remove(0);
    assert!(
        resized.contains(&geometry(10.0, 5.0, 30.0, 20.0)),
        "{resized}"
    );
    assert_eq!(shot(&doc, "I9:11;9:3", 1.0), "30x20 px, scale 1");
    // The scaled instance states the final size itself; only the inherited offset scales.
    let stated = texts(&metadata(&doc, Some("I9:12;9:3"), Some(0))).remove(0);
    assert!(
        stated.contains(&geometry(20.0, 10.0, 61.0, 41.0)),
        "{stated}"
    );
    assert_eq!(shot(&doc, "I9:12;9:3", 1.0), "61x41 px, scale 1");
}

#[test]
fn design_context_scales_child_sizes_offsets_and_strokes() {
    let doc = document();
    let options = Options {
        file_key: "structure".into(),
        assets: Assets::Inline,
        ..Default::default()
    };
    let out = texts(&design_context(&doc, "9:10", &options));
    let line = |id: &str| {
        out[0]
            .lines()
            .find(|l| l.contains(&format!("data-node-id=\"{id}\"")))
            .unwrap_or_else(|| panic!("{id} missing: {}", out[0]))
            .to_owned()
    };
    let child = line("I9:10;9:3");
    for class in [
        "left-[20px] top-[10px]",
        "w-[60px] h-[40px]",
        "border-[4px]",
    ] {
        assert!(child.contains(class), "{class}: {child}");
    }
    // The radius is drawn unscaled by the renderer too; the output says so.
    assert!(child.contains("rounded-[4px]"), "{child}");
    assert!(
        out[1].contains("2 lengths kept unscaled in scaled instances"),
        "{}",
        out[1]
    );
    let nested = line("I9:10;9:6;9:5");
    assert!(
        nested.contains("left-[6px] top-[9px]") && nested.contains("w-[18px] h-[12px]"),
        "{nested}"
    );
    let resized = texts(&design_context(&doc, "9:11", &options));
    assert!(
        resized[0].contains("left-[10px] top-[5px] w-[30px] h-[20px]"),
        "{}",
        resized[0]
    );
    // Only the nested 1.5x instance's rectangle is inside a uniform scale.
    assert!(
        resized[1].contains("1 lengths kept unscaled"),
        "{}",
        resized[1]
    );
}

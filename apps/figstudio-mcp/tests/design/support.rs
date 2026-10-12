//! Documents built in the test for get_design_context.
use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp::tools::design::{Assets, Options, design_context};
use rmcp::model::CallToolResult;
use std::path::PathBuf;

pub fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig")
}
pub fn obj(fields: &[(&str, Val)]) -> Val {
    let mut out = Obj::new();
    for (key, value) in fields {
        out.insert(key, value.clone());
    }
    Val::Obj(out)
}
pub fn f(v: f64) -> Val {
    Val::Float(v)
}
pub fn e(v: &'static str) -> Val {
    Val::Enum(v)
}
pub fn rgba(r: f64, g: f64, b: f64, a: f64) -> Val {
    obj(&[("r", f(r)), ("g", f(g)), ("b", f(b)), ("a", f(a))])
}
pub fn solid(color: Val) -> Val {
    obj(&[("type", e("SOLID")), ("color", color)])
}
pub fn at(x: f64, y: f64) -> Val {
    obj(&[
        ("m00", f(1.0)),
        ("m11", f(1.0)),
        ("m02", f(x)),
        ("m12", f(y)),
    ])
}
/// A node `9:id` under `parent` (the page is 0:1), 10×10 at (0, 0) unless `fields` set it.
pub fn node(id: u32, parent: Guid, kind: &str, name: &str, fields: &[(&str, Val)]) -> Node {
    let mut data = obj(&[
        ("guid", Guid(9, id).to_val()),
        (
            "parentIndex",
            obj(&[
                ("guid", parent.to_val()),
                ("position", Val::Str(format!("{id:04}"))),
            ]),
        ),
        ("type", Val::Str(kind.into())),
        ("name", Val::Str(name.into())),
        ("size", obj(&[("x", f(10.0)), ("y", f(10.0))])),
    ]);
    for (key, value) in fields {
        data.set(key, value.clone());
    }
    Node::from_data(data).unwrap()
}
pub const PAGE: Guid = Guid(0, 1);
pub fn g(id: u32) -> Guid {
    Guid(9, id)
}
/// structure.fig with `nodes` added.
pub fn doc_with(nodes: Vec<Node>) -> Document {
    let mut doc = Document::open(fixture()).unwrap();
    let mut all = doc.nodes.clone();
    all.extend(nodes);
    doc.set_nodes(all).unwrap();
    doc
}
pub fn inline() -> Options {
    Options {
        file_key: "structure".into(),
        assets: Assets::Inline,
        ..Default::default()
    }
}
pub fn texts(result: &CallToolResult) -> Vec<String> {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect()
}
/// The code block of a successful call.
pub fn code(doc: &Document, id: &str) -> String {
    let result = design_context(doc, id, &inline());
    assert_ne!(result.is_error, Some(true), "{:?}", texts(&result));
    texts(&result).remove(0)
}
/// Instructions, assets and approximations of a successful call.
pub fn notes(doc: &Document, id: &str) -> String {
    let result = design_context(doc, id, &inline());
    assert_ne!(result.is_error, Some(true), "{:?}", texts(&result));
    texts(&result).remove(1)
}

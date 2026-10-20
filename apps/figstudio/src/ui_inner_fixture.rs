//! Synthetic definitions are built from the public fixture schema, never private content.
use super::pick::object as obj;
use super::*;
use figstudio_fig::{Document, Node, Val};
use std::sync::Arc;

pub fn id(n: u32) -> Guid {
    Guid(99, n)
}
fn node(doc: &Document, n: u32, parent: u32, kind: &str, x: f64, y: f64, size: f64) -> Node {
    let mut data = doc.node(pick::TEXT).unwrap().data.clone();
    data.set("guid", id(n).to_val());
    data.set("type", Val::Str(kind.into()));
    data.get_mut("parentIndex")
        .unwrap()
        .set("guid", id(parent).to_val());
    data.get_mut("parentIndex")
        .unwrap()
        .set("position", Val::Str(format!("{n:03}")));
    let tf = data.get_mut("transform").unwrap();
    tf.set("m02", Val::Float(x));
    tf.set("m12", Val::Float(y));
    data.set(
        "size",
        obj(&[("x", Val::Float(size)), ("y", Val::Float(size))]),
    );
    Node::from_data(data).unwrap()
}

pub fn document() -> Document {
    let mut doc = pick::nested();
    let mut nodes = vec![
        node(&doc, 1, 0, "CANVAS", 0.0, 0.0, 400.0),
        node(&doc, 2, 1, "FRAME", 0.0, 0.0, 300.0),
        node(&doc, 100, 0, "SYMBOL", 0.0, 0.0, 240.0),
        node(&doc, 101, 100, "GROUP", 0.0, 0.0, 240.0),
        node(&doc, 102, 101, "GROUP", 0.0, 0.0, 80.0),
        node(&doc, 103, 102, "TEXT", 0.0, 0.0, 60.0),
        node(&doc, 104, 101, "GROUP", 100.0, 0.0, 80.0),
        node(&doc, 105, 104, "RECTANGLE", 0.0, 0.0, 60.0),
        node(&doc, 106, 101, "INSTANCE", 0.0, 100.0, 80.0),
        node(&doc, 110, 0, "SYMBOL", 0.0, 0.0, 80.0),
        node(&doc, 111, 110, "GROUP", 0.0, 0.0, 80.0),
        node(&doc, 112, 111, "TEXT", 0.0, 0.0, 60.0),
        node(&doc, 200, 2, "INSTANCE", 0.0, 0.0, 240.0),
    ];
    nodes[8]
        .data
        .set("symbolData", obj(&[("symbolID", id(110).to_val())]));
    nodes[12]
        .data
        .set("symbolData", obj(&[("symbolID", id(100).to_val())]));
    nodes.push(node(&doc, 107, 102, "RECTANGLE", 65.0, 0.0, 10.0));
    doc.set_nodes(nodes).unwrap();
    doc
}

pub fn path(keys: &[u32]) -> figstudio_fig::resolve::InstancePath {
    figstudio_fig::resolve::InstancePath::new(
        std::iter::once(id(200))
            .chain(keys.iter().map(|n| id(*n)))
            .collect(),
    )
    .unwrap()
}

pub fn harness() -> Harness<'static, App> {
    let mut h = pick::nested_harness();
    h.state_mut().tabs[0].doc = Arc::new(document());
    h.state_mut().tabs[0].fit_pending = true;
    h.run_steps(4);
    h
}

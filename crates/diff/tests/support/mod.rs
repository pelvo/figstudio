#![allow(dead_code, reason = "each test suite uses a subset of these builders")]

use figstudio_fig::{Document, Guid, Node, Obj, Val};

pub fn node(id: u32, parent: Option<u32>, kind: &str, position: &str) -> Node {
    let mut data = Val::Obj(Obj::from([
        ("guid", Guid(0, id).to_val()),
        ("type", Val::Str(kind.into())),
        ("name", Val::Str(format!("layer-{id}"))),
    ]));
    if let Some(parent) = parent {
        data.set(
            "parentIndex",
            Val::Obj(Obj::from([
                ("guid", Guid(0, parent).to_val()),
                ("position", Val::Str(position.into())),
            ])),
        );
    }
    Node::from_data(data).unwrap()
}

pub fn document(extra: Vec<Node>) -> Document {
    // Reuse only the public fixture's container/schema; all compared nodes are synthetic.
    let mut doc =
        Document::from_bytes(include_bytes!("../../../../tests/fixtures/structure.fig")).unwrap();
    let mut nodes = vec![
        node(0, None, "DOCUMENT", ""),
        node(1, Some(0), "CANVAS", "a"),
    ];
    nodes.extend(extra);
    doc.set_nodes(nodes).unwrap();
    doc
}

pub fn rectangle(id: u32, parent: u32) -> Node {
    node(id, Some(parent), "RECTANGLE", "a")
}

pub fn solid(r: f64, g: f64, b: f64) -> Val {
    Val::Arr(vec![object(&[
        ("type", Val::Str("SOLID".into())),
        (
            "color",
            object(&[
                ("r", Val::Float(r)),
                ("g", Val::Float(g)),
                ("b", Val::Float(b)),
                ("a", Val::Float(1.0)),
            ]),
        ),
    ])])
}

/// A solid-filled rectangle; origin is `(x, y)` in parent space.
pub fn filled_rectangle_at(
    id: u32,
    parent: u32,
    rgb: (f64, f64, f64),
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Node {
    let mut rect = rectangle(id, parent);
    rect.data.set(
        "size",
        object(&[("x", Val::Float(w)), ("y", Val::Float(h))]),
    );
    rect.data.set("fillPaints", solid(rgb.0, rgb.1, rgb.2));
    if x != 0.0 || y != 0.0 {
        rect.data.set(
            "transform",
            object(&[("m02", Val::Float(x)), ("m12", Val::Float(y))]),
        );
    }
    Node::from_data(rect.data).unwrap()
}

/// A 40x40 rectangle at the page origin with an opaque solid fill.
pub fn filled_rectangle(id: u32, parent: u32, rgb: (f64, f64, f64)) -> Node {
    filled_rectangle_at(id, parent, rgb, 0.0, 0.0, 40.0, 40.0)
}

/// Two pages (ids 1 and 11); `first`/`second` are their child nodes.
pub fn two_page_document(first: Vec<Node>, second: Vec<Node>) -> Document {
    // Reuse only the public fixture's container/schema; all compared nodes are synthetic.
    let mut doc =
        Document::from_bytes(include_bytes!("../../../../tests/fixtures/structure.fig")).unwrap();
    let mut nodes = vec![
        node(0, None, "DOCUMENT", ""),
        node(1, Some(0), "CANVAS", "a"),
        node(11, Some(0), "CANVAS", "b"),
    ];
    nodes.extend(first);
    nodes.extend(second);
    doc.set_nodes(nodes).unwrap();
    doc
}

pub fn object(fields: &[(&str, Val)]) -> Val {
    let mut out = Obj::new();
    for (field, value) in fields {
        out.insert(field, value.clone());
    }
    Val::Obj(out)
}

/// A 40x40 rectangle whose only fill is the image with this 40-digit lowercase hex hash.
pub fn image_rectangle(id: u32, parent: u32, hash: &str) -> Node {
    let bytes: Vec<u8> = (0..hash.len() / 2)
        .map(|i| u8::from_str_radix(&hash[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    let mut rect = filled_rectangle(id, parent, (0.0, 0.0, 0.0));
    let paint = object(&[
        ("type", Val::Enum("IMAGE")),
        ("visible", Val::Bool(true)),
        ("opacity", Val::Float(1.0)),
        ("image", object(&[("hash", Val::Bytes(bytes))])),
        ("imageScaleMode", Val::Enum("STRETCH")),
    ]);
    rect.data.set("fillPaints", Val::Arr(vec![paint]));
    Node::from_data(rect.data).unwrap()
}

/// Pages 1, 11, 21, ... one per colour, each holding one filled rectangle.
pub fn many_page_document(colors: &[(f64, f64, f64)]) -> Document {
    let mut doc =
        Document::from_bytes(include_bytes!("../../../../tests/fixtures/structure.fig")).unwrap();
    let mut nodes = vec![node(0, None, "DOCUMENT", "")];
    for (i, rgb) in colors.iter().enumerate() {
        let page = 1 + 10 * i as u32;
        nodes.push(node(page, Some(0), "CANVAS", &format!("{i:02}")));
        nodes.push(filled_rectangle(page + 1, page, *rgb));
    }
    doc.set_nodes(nodes).unwrap();
    doc
}

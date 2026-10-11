#![allow(
    dead_code,
    reason = "each integration suite uses a different subset of these builders"
)]
use std::path::PathBuf;

use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_render::{Renderer, Viewport};
use vello_cpu::kurbo::Affine;

pub fn obj(fields: &[(&str, Val)]) -> Val {
    Val::Obj(fields.iter().fold(Obj::new(), |mut o, (k, v)| {
        o.insert(k, v.clone());
        o
    }))
}
pub fn id(n: u32) -> Guid {
    Guid(99, n)
}
pub fn size(w: f64, h: f64) -> Val {
    obj(&[("x", Val::Float(w)), ("y", Val::Float(h))])
}
pub fn node(n: u32, parent: u32, kind: &str) -> Node {
    Node::from_data(obj(&[
        ("guid", id(n).to_val()),
        ("type", Val::Str(kind.into())),
        (
            "parentIndex",
            obj(&[
                ("guid", id(parent).to_val()),
                ("position", Val::Str(format!("{n:03}"))),
            ]),
        ),
        ("size", size(40.0, 40.0)),
        ("frameMaskDisabled", Val::Bool(true)),
    ]))
    .unwrap()
}
pub fn paint(r: f64, g: f64, b: f64) -> Val {
    Val::Arr(vec![obj(&[
        ("type", Val::Str("SOLID".into())),
        (
            "color",
            obj(&[
                ("r", Val::Float(r)),
                ("g", Val::Float(g)),
                ("b", Val::Float(b)),
                ("a", Val::Float(1.0)),
            ]),
        ),
    ])])
}
pub fn patch(path: &[u32], fields: &[(&str, Val)]) -> Val {
    let mut v = obj(fields);
    v.set(
        "guidPath",
        obj(&[(
            "guids",
            Val::Arr(path.iter().map(|n| id(*n).to_val()).collect()),
        )]),
    );
    v
}
pub fn instance(n: u32, parent: u32, symbol: u32, patches: Vec<Val>) -> Node {
    let mut n = node(n, parent, "INSTANCE");
    n.data.set(
        "symbolData",
        obj(&[
            ("symbolID", id(symbol).to_val()),
            ("symbolOverrides", Val::Arr(patches)),
        ]),
    );
    n
}
pub fn document(nodes: Vec<Node>) -> Document {
    let mut doc = Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
    )
    .unwrap();
    doc.set_nodes(nodes).unwrap();
    doc
}
pub fn pixels(doc: &Document) -> Vec<u8> {
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    renderer
        .render(
            doc,
            id(1),
            &Viewport {
                width: 80,
                height: 80,
                transform: Affine::IDENTITY,
                background: None,
            },
        )
        .pixmap
        .data_as_u8_slice()
        .to_vec()
}
pub fn pixel(px: &[u8], x: usize, y: usize) -> &[u8] {
    &px[(y * 80 + x) * 4..(y * 80 + x) * 4 + 4]
}

/// A frame clipping a shadowed, blurred, half-transparent, stroked rectangle that straddles
/// the tiles around page (0, 0), and text, at negative tile coordinates.
pub fn effects_doc() -> Document {
    let mut frame = node(2, 1, "FRAME");
    frame.data.set("frameMaskDisabled", Val::Bool(false));
    frame.data.set("size", size(400.0, 400.0));
    frame.data.set(
        "transform",
        obj(&[("m02", Val::Float(-200.0)), ("m12", Val::Float(-200.0))]),
    );
    frame.data.set("fillPaints", paint(0.9, 0.9, 1.0));
    let mut rect = node(3, 2, "RECTANGLE");
    rect.data.set("size", size(240.0, 240.0));
    rect.data.set(
        "transform",
        obj(&[("m02", Val::Float(60.0)), ("m12", Val::Float(60.0))]),
    );
    rect.data.set("fillPaints", paint(1.0, 0.2, 0.2));
    rect.data.set("strokePaints", paint(0.0, 0.0, 0.6));
    rect.data.set("strokeWeight", Val::Float(6.0));
    rect.data.set("opacity", Val::Float(0.7));
    let effect = |kind: &str, dx: f64| {
        obj(&[
            ("type", Val::Str(kind.into())),
            ("visible", Val::Bool(true)),
            ("color", obj(&[("a", Val::Float(0.8))])),
            (
                "offset",
                obj(&[("x", Val::Float(dx)), ("y", Val::Float(dx))]),
            ),
            ("radius", Val::Float(24.0)),
        ])
    };
    rect.data
        .set("effects", Val::Arr(vec![effect("DROP_SHADOW", 30.0)]));
    let mut blurred = node(4, 1, "ELLIPSE");
    blurred.data.set("size", size(120.0, 120.0));
    blurred.data.set(
        "transform",
        obj(&[("m02", Val::Float(-60.0)), ("m12", Val::Float(250.0))]),
    );
    blurred.data.set("fillPaints", paint(0.1, 0.6, 0.1));
    blurred
        .data
        .set("effects", Val::Arr(vec![effect("LAYER_BLUR", 0.0)]));
    // Left of tile column 0 (page x >= -2 at scale 1): only their shadow and blur reach it.
    let mut shadowed = node(5, 1, "RECTANGLE");
    shadowed.data.set("size", size(60.0, 60.0));
    shadowed.data.set(
        "transform",
        obj(&[("m02", Val::Float(-90.0)), ("m12", Val::Float(420.0))]),
    );
    shadowed.data.set("fillPaints", paint(0.2, 0.2, 0.2));
    shadowed
        .data
        .set("effects", Val::Arr(vec![effect("DROP_SHADOW", 50.0)]));
    let mut soft = node(6, 1, "RECTANGLE");
    soft.data.set("size", size(60.0, 60.0));
    soft.data.set(
        "transform",
        obj(&[("m02", Val::Float(-70.0)), ("m12", Val::Float(-400.0))]),
    );
    soft.data.set("fillPaints", paint(0.0, 0.0, 0.0));
    soft.data
        .set("effects", Val::Arr(vec![effect("LAYER_BLUR", 0.0)]));
    document(vec![
        node(1, 0, "CANVAS"),
        frame,
        rect,
        blurred,
        shadowed,
        soft,
    ])
}

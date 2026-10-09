//! Synthetic instance regressions: no private document data.
use std::path::PathBuf;

use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_render::{Renderer, Viewport};
use vello_cpu::kurbo::Affine;

fn obj(fields: &[(&str, Val)]) -> Val {
    Val::Obj(fields.iter().fold(Obj::new(), |mut o, (k, v)| {
        o.insert(k, v.clone());
        o
    }))
}
fn id(n: u32) -> Guid {
    Guid(99, n)
}
fn size(w: f64, h: f64) -> Val {
    obj(&[("x", Val::Float(w)), ("y", Val::Float(h))])
}
fn node(n: u32, parent: u32, kind: &str) -> Node {
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
fn paint(r: f64, g: f64, b: f64) -> Val {
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
fn patch(path: &[u32], fields: &[(&str, Val)]) -> Val {
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
fn instance(n: u32, parent: u32, symbol: u32, patches: Vec<Val>) -> Node {
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
fn document(nodes: Vec<Node>) -> Document {
    let mut doc = Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
    )
    .unwrap();
    doc.set_nodes(nodes).unwrap();
    doc
}
fn pixels(doc: &Document) -> Vec<u8> {
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
fn pixel(px: &[u8], x: usize, y: usize) -> &[u8] {
    &px[(y * 80 + x) * 4..(y * 80 + x) * 4 + 4]
}

#[test]
fn override_paths_skip_frames_but_keep_instance_ancestors() {
    let mut rect = node(5, 4, "RECTANGLE");
    rect.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        node(3, 2, "FRAME"),
        instance(6, 3, 7, vec![]),
        node(7, 0, "SYMBOL"),
        node(4, 7, "FRAME"),
        rect,
        instance(
            8,
            1,
            2,
            vec![patch(&[6, 5], &[("fillPaints", paint(0.0, 1.0, 0.0))])],
        ),
    ]);
    assert_eq!(pixel(&pixels(&doc), 10, 10), &[0, 255, 0, 255]);
}

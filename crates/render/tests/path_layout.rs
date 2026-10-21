mod common;
use common::*;
use figstudio_fig::{Val, resolve::InstancePath};
use figstudio_render::{
    Renderer, geom,
    kurbo::{Point, Rect},
};

#[test]
fn resolved_layout_removes_only_stale_glyphs_without_changing_the_document() {
    let mut text = node(4, 2, "TEXT");
    let characters = |s: &str| obj(&[("characters", Val::Str(s.into()))]);
    text.data.set("textData", characters("base"));
    let glyphs = obj(&[(
        "glyphs",
        Val::Arr(vec![obj(&[("fontSize", Val::Float(10.0))])]),
    )]);
    text.data.set("derivedTextData", glyphs.clone());
    let top = instance(
        3,
        1,
        2,
        vec![patch(&[4], &[("textData", characters("override"))])],
    );
    let mut doc = document(vec![node(1, 0, "CANVAS"), node(2, 0, "SYMBOL"), text, top]);
    let path = InstancePath::new(vec![id(3), id(4)]).unwrap();
    let before = doc.nodes.clone();
    let (_, data) = Renderer::new().path_layout(&doc, &path).unwrap();
    assert_eq!(
        data.path(&["textData", "characters"]).and_then(Val::as_str),
        Some("override")
    );
    assert!(data.get("derivedTextData").is_none());
    assert_eq!(doc.node(id(4)).unwrap().data, before[2].data);
    let mut nodes = doc.nodes.clone();
    nodes[3].data.set(
        "derivedSymbolData",
        Val::Arr(vec![patch(&[4], &[("derivedTextData", glyphs.clone())])]),
    );
    doc.set_nodes(nodes).unwrap();
    let (_, fresh) = Renderer::new().path_layout(&doc, &path).unwrap();
    assert_eq!(fresh.get("derivedTextData"), Some(&glyphs));
    // Equal glyph values can still be an explicitly fresh derived override.
    let plain = InstancePath::new(vec![id(3), id(99)]).unwrap();
    assert!(Renderer::new().path_layout(&doc, &plain).is_none());
}

#[test]
fn layout_and_bounds_share_resized_rotated_nested_instance_geometry() {
    let mut rect = node(4, 3, "RECTANGLE");
    rect.data.set("size", size(10.0, 10.0));
    rect.data.set("fillPaints", paint(0.0, 1.0, 0.0));
    rect.data
        .set("horizontalConstraint", Val::Str("CENTER".into()));
    rect.data
        .set("verticalConstraint", Val::Str("CENTER".into()));
    rect.data.set(
        "transform",
        obj(&[
            ("m00", Val::Float(0.0)),
            ("m01", Val::Float(-1.0)),
            ("m10", Val::Float(1.0)),
            ("m11", Val::Float(0.0)),
            ("m02", Val::Float(20.0)),
            ("m12", Val::Float(10.0)),
        ]),
    );
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        node(3, 2, "FRAME"),
        rect,
        instance(5, 7, 2, vec![patch(&[3], &[("size", size(80.0, 80.0))])]),
        node(7, 0, "SYMBOL"),
        instance(8, 1, 7, vec![]),
    ]);
    let path = InstancePath::new(vec![id(8), id(5), id(4)]).unwrap();
    let mut renderer = Renderer::new();
    let (tf, data) = renderer.path_layout(&doc, &path).unwrap();
    assert_eq!(tf * Point::new(0.0, 0.0), Point::new(40.0, 30.0));
    let (w, h) = geom::size(&data);
    assert_eq!(
        renderer.path_bounds(&doc, &path),
        Some(geom::transformed_bounds(tf, Rect::new(0.0, 0.0, w, h)))
    );
    assert_eq!(pixel(&pixels(&doc), 35, 35), &[0, 255, 0, 255]);
    assert_eq!(pixel(&pixels(&doc), 15, 15), &[0, 0, 0, 0]);
}

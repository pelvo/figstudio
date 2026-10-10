//! Synthetic override path and geometry regressions.
mod common;
use common::*;
use figstudio_fig::Val;

#[test]
fn uniform_instance_scale_updates_omitted_translation_but_not_precomputed_geometry() {
    let mut rect = node(4, 3, "RECTANGLE");
    rect.data.set("size", size(10.0, 10.0));
    rect.data.set("fillPaints", paint(0.0, 1.0, 0.0));
    rect.data.set(
        "transform",
        obj(&[
            ("m00", Val::Float(-1.0)),
            ("m02", Val::Float(15.0)),
            ("m12", Val::Float(5.0)),
        ]),
    );
    let mut top = instance(5, 1, 2, vec![]);
    top.data
        .get_mut("symbolData")
        .unwrap()
        .set("uniformScaleFactor", Val::Float(2.0));
    let mut doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        node(3, 2, "FRAME"),
        rect,
        top.clone(),
    ]);
    let blob = doc.blobs.len() as i64;
    let mut commands = Vec::new();
    for (cmd, x, y) in [
        (1, 0f32, 0f32),
        (2, 10.0, 0.0),
        (2, 10.0, 20.0),
        (2, 0.0, 20.0),
    ] {
        commands.push(cmd);
        commands.extend(x.to_le_bytes());
        commands.extend(y.to_le_bytes());
    }
    commands.push(0);
    doc.blobs.push(commands);
    top.data.set(
        "derivedSymbolData",
        Val::Arr(vec![patch(
            &[4],
            &[
                ("size", size(10.0, 20.0)),
                (
                    "fillGeometry",
                    Val::Arr(vec![obj(&[("commandsBlob", Val::Int(blob))])]),
                ),
            ],
        )]),
    );
    let mut nodes = doc.nodes.clone();
    *nodes.iter_mut().find(|n| n.id == id(5)).unwrap() = top;
    doc.set_nodes(nodes).unwrap();
    let px = pixels(&doc);
    assert_eq!(pixel(&px, 25, 25), &[0, 255, 0, 255]);
    assert_eq!(pixel(&px, 15, 15), &[0, 0, 0, 0]);
    assert_eq!(pixel(&px, 25, 35), &[0, 0, 0, 0]);
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

#[test]
fn resized_frames_move_centered_children_without_replacing_rotations_or_fresh_transforms() {
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
    let top = instance(5, 1, 2, vec![patch(&[3], &[("size", size(80.0, 80.0))])]);
    let mut doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        node(3, 2, "FRAME"),
        rect,
        top,
    ]);
    let px = pixels(&doc);
    assert_eq!(pixel(&px, 35, 35), &[0, 255, 0, 255]);
    assert_eq!(pixel(&px, 15, 15), &[0, 0, 0, 0]);
    let mut nodes = doc.nodes.clone();
    nodes.iter_mut().find(|n| n.id == id(5)).unwrap().data.set(
        "derivedSymbolData",
        Val::Arr(vec![patch(
            &[4],
            &[(
                "transform",
                obj(&[("m02", Val::Float(50.0)), ("m12", Val::Float(50.0))]),
            )],
        )]),
    );
    doc.set_nodes(nodes).unwrap();
    assert_eq!(pixel(&pixels(&doc), 55, 55), &[0, 255, 0, 255]);
}

#[test]
fn uniform_scale_applies_to_values_stated_by_nested_instances_but_not_by_itself() {
    let mut inner_rect = node(5, 4, "RECTANGLE");
    inner_rect.data.set("fillPaints", paint(0.0, 1.0, 0.0));
    let mut outer_rect = node(6, 4, "RECTANGLE");
    outer_rect.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    let at = |x: f64, y: f64| obj(&[("m02", Val::Float(x)), ("m12", Val::Float(y))]);
    let mut nested = instance(3, 2, 4, vec![]);
    // The nested instance restates its child's place and size in its own, unscaled space.
    nested.data.set(
        "derivedSymbolData",
        Val::Arr(vec![patch(
            &[5],
            &[("transform", at(5.0, 5.0)), ("size", size(10.0, 10.0))],
        )]),
    );
    let mut top = instance(7, 1, 2, vec![]);
    top.data
        .get_mut("symbolData")
        .unwrap()
        .set("uniformScaleFactor", Val::Float(2.0));
    // The scaled instance itself states the other child already in scaled space.
    top.data.set(
        "derivedSymbolData",
        Val::Arr(vec![patch(
            &[3, 6],
            &[("transform", at(50.0, 50.0)), ("size", size(10.0, 10.0))],
        )]),
    );
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        nested,
        node(4, 0, "SYMBOL"),
        inner_rect,
        outer_rect,
        top,
    ]);
    let px = pixels(&doc);
    assert_eq!(pixel(&px, 11, 11), &[0, 255, 0, 255]);
    assert_eq!(pixel(&px, 28, 28), &[0, 255, 0, 255]);
    assert_eq!(pixel(&px, 32, 32), &[0, 0, 0, 0]);
    assert_eq!(pixel(&px, 55, 55), &[255, 0, 0, 255]);
    assert_eq!(pixel(&px, 62, 62), &[0, 0, 0, 0]);
}

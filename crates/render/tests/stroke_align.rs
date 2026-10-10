//! Stored stroke geometry for inside/outside strokes is twice the weight and must be clipped.
mod common;
use common::*;
use figstudio_fig::{Document, Val};

/// A 40x40 frame at (20,20) with a 2-unit stroke drawn the way Figma stores it for an
/// inside or outside alignment: a centred 4-unit band around the edge (no stored geometry,
/// so the renderer strokes the base shape itself).
fn stroked(align: &str) -> Document {
    let mut frame = node(2, 1, "FRAME");
    frame.data.set(
        "transform",
        obj(&[("m02", Val::Float(20.0)), ("m12", Val::Float(20.0))]),
    );
    frame.data.set("strokePaints", paint(1.0, 0.0, 0.0));
    frame.data.set("strokeWeight", Val::Float(2.0));
    frame.data.set("strokeAlign", Val::Str(align.into()));
    document(vec![node(1, 0, "CANVAS"), frame])
}

fn rect_cmds(out: &mut Vec<u8>, x0: f32, y0: f32, x1: f32, y1: f32) {
    for (cmd, x, y) in [(1, x0, y0), (2, x1, y0), (2, x1, y1), (2, x0, y1)] {
        out.push(cmd);
        out.extend(x.to_le_bytes());
        out.extend(y.to_le_bytes());
    }
    out.push(0);
}

/// Same frame with stored geometry: the fill rectangle and a 4-unit centred band (even-odd).
fn stored(align: &str) -> Document {
    let mut doc = stroked(align);
    let (fill, band) = (doc.blobs.len() as i64, doc.blobs.len() as i64 + 1);
    let mut f = Vec::new();
    rect_cmds(&mut f, 0.0, 0.0, 40.0, 40.0);
    let mut b = Vec::new();
    rect_cmds(&mut b, -2.0, -2.0, 42.0, 42.0);
    rect_cmds(&mut b, 2.0, 2.0, 38.0, 38.0);
    doc.blobs.extend([f, b]);
    let mut nodes = doc.nodes.clone();
    let frame = &mut nodes.iter_mut().find(|n| n.id == id(2)).unwrap().data;
    let geom = |blob, rule: &str| {
        Val::Arr(vec![obj(&[
            ("commandsBlob", Val::Int(blob)),
            ("windingRule", Val::Str(rule.into())),
        ])])
    };
    frame.set("fillGeometry", geom(fill, "NONZERO"));
    frame.set("strokeGeometry", geom(band, "ODD"));
    doc.set_nodes(nodes).unwrap();
    doc
}

fn red(px: &[u8], x: usize, y: usize) -> bool {
    pixel(px, x, y) == [255, 0, 0, 255]
}

#[test]
fn inside_strokes_stay_within_the_shape_at_their_weight() {
    let px = pixels(&stroked("INSIDE"));

    assert!(
        red(&px, 20, 40) && red(&px, 21, 40),
        "2-unit band inside the edge"
    );
    assert!(!red(&px, 19, 40), "nothing outside the edge");
    assert!(!red(&px, 23, 40), "band is the weight, not twice it");
}

#[test]
fn outside_strokes_stay_outside_the_shape_at_their_weight() {
    let px = pixels(&stroked("OUTSIDE"));

    assert!(
        red(&px, 18, 40) && red(&px, 19, 40),
        "2-unit band outside the edge"
    );
    assert!(!red(&px, 20, 40), "nothing inside the edge");
    assert!(!red(&px, 16, 40), "band is the weight, not twice it");
}

#[test]
fn stored_inside_and_outside_stroke_geometry_is_clipped_to_the_weight() {
    let inside = pixels(&stored("INSIDE"));
    let outside = pixels(&stored("OUTSIDE"));

    assert!(red(&inside, 20, 40) && red(&inside, 21, 40));
    assert!(!red(&inside, 18, 40) && !red(&inside, 19, 40));
    assert!(red(&outside, 18, 40) && red(&outside, 19, 40));
    assert!(!red(&outside, 20, 40) && !red(&outside, 21, 40));
}

#[test]
fn center_strokes_straddle_the_edge() {
    let px = pixels(&stroked("CENTER"));

    assert!(red(&px, 19, 40) && red(&px, 20, 40));
    assert!(!red(&px, 18, 40) && !red(&px, 21, 40));
}

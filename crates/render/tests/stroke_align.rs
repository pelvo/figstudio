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

/// Vector network blob (header, vertices, segments, regions) for a closed square, or for the
/// same four segments with no region loop (an open path).
fn square_network(closed: bool) -> Vec<u8> {
    let mut b = Vec::new();
    let u = |b: &mut Vec<u8>, v: u32| b.extend(v.to_le_bytes());
    let (regions, corners) = (
        u32::from(closed),
        [(0.0f32, 0.0f32), (40.0, 0.0), (40.0, 40.0), (0.0, 40.0)],
    );
    for v in [4, 4, regions] {
        u(&mut b, v);
    }
    for (x, y) in corners {
        u(&mut b, 0);
        b.extend(x.to_le_bytes());
        b.extend(y.to_le_bytes());
    }
    for s in 0..4u32 {
        u(&mut b, 0);
        u(&mut b, s);
        b.extend([0u8; 8]);
        u(&mut b, (s + 1) % 4);
        b.extend([0u8; 8]);
    }
    if closed {
        for v in [0, 1, 4, 0, 1, 2, 3] {
            u(&mut b, v);
        }
    }
    b
}

/// The stored-geometry frame turned into a VECTOR with a vector network.
fn stored_vector(align: &str, closed: bool) -> Document {
    let mut doc = stored(align);
    let blob = doc.blobs.len() as i64;
    doc.blobs.push(square_network(closed));
    let mut nodes = doc.nodes.clone();
    let vector = nodes.iter_mut().find(|n| n.id == id(2)).unwrap();
    vector.kind = "VECTOR".into();
    vector.data.set("type", Val::Str("VECTOR".into()));
    vector.data.set("size", size(40.0, 40.0));
    vector
        .data
        .set("vectorData", obj(&[("vectorNetworkBlob", Val::Int(blob))]));
    doc.set_nodes(nodes).unwrap();
    doc
}

#[test]
fn closed_vector_inside_and_outside_strokes_are_clipped_to_the_weight() {
    let inside = pixels(&stored_vector("INSIDE", true));
    let outside = pixels(&stored_vector("OUTSIDE", true));

    assert!(
        red(&inside, 20, 40) && red(&inside, 21, 40),
        "inside band drawn"
    );
    assert!(
        !red(&inside, 18, 40) && !red(&inside, 19, 40),
        "inside stroke of a closed vector stays within its fill"
    );
    assert!(
        red(&outside, 18, 40) && red(&outside, 19, 40),
        "outside band drawn"
    );
    assert!(
        !red(&outside, 20, 40) && !red(&outside, 21, 40),
        "outside stroke of a closed vector stays outside its fill"
    );
}

#[test]
fn open_vector_strokes_are_left_unclipped() {
    let px = pixels(&stored_vector("INSIDE", false));

    assert!(
        red(&px, 18, 40) && red(&px, 21, 40),
        "an open path has no inside, so the stored band is drawn whole"
    );
}

/// Text with one 20-unit square glyph at (20,20) and a stored 4-unit centred band around it,
/// the way Figma stores a 2-unit inside stroke on text.
#[test]
fn stored_inside_text_stroke_is_clipped_to_the_glyphs() {
    let mut text = node(2, 1, "TEXT");
    text.data.set("fillPaints", paint(0.0, 0.0, 1.0));
    text.data.set("strokePaints", paint(1.0, 0.0, 0.0));
    text.data.set("strokeWeight", Val::Float(2.0));
    text.data.set("strokeAlign", Val::Str("INSIDE".into()));
    let mut doc = document(vec![node(1, 0, "CANVAS"), text]);
    let (glyph, band) = (doc.blobs.len() as i64, doc.blobs.len() as i64 + 1);
    let mut g = Vec::new();
    rect_cmds(&mut g, 0.0, 0.0, 1.0, 1.0);
    let mut b = Vec::new();
    rect_cmds(&mut b, 18.0, 18.0, 42.0, 42.0);
    rect_cmds(&mut b, 22.0, 22.0, 38.0, 38.0);
    doc.blobs.extend([g, b]);
    let mut nodes = doc.nodes.clone();
    let data = &mut nodes.iter_mut().find(|n| n.id == id(2)).unwrap().data;
    // Glyph outlines are y-up from the baseline: baseline y 40 covers y 20..40.
    let glyphs = obj(&[(
        "glyphs",
        Val::Arr(vec![obj(&[
            ("commandsBlob", Val::Int(glyph)),
            ("fontSize", Val::Float(20.0)),
            ("position", size(20.0, 40.0)),
        ])]),
    )]);
    data.set("derivedTextData", glyphs);
    data.set(
        "strokeGeometry",
        Val::Arr(vec![obj(&[
            ("commandsBlob", Val::Int(band)),
            ("windingRule", Val::Str("ODD".into())),
        ])]),
    );
    doc.set_nodes(nodes).unwrap();
    let px = pixels(&doc);

    assert!(
        red(&px, 20, 30) && red(&px, 21, 30),
        "band inside the glyph"
    );
    assert!(
        !red(&px, 18, 30) && !red(&px, 19, 30),
        "nothing outside the glyph"
    );
}

//! Stroke join, cap, dash pattern and per-side border weights (KNOWN-GAPS A11).
mod common;
use common::*;
use figstudio_fig::{Document, Val};

/// A 40x40 frame at (20,20) with a red stroke and no stored stroke geometry.
fn frame(weight: f64, align: &str, fields: &[(&str, Val)]) -> figstudio_fig::Node {
    framed(1, weight, align, fields)
}

fn framed(parent: u32, weight: f64, align: &str, fields: &[(&str, Val)]) -> figstudio_fig::Node {
    let mut frame = node(2, parent, "FRAME");
    frame.data.set(
        "transform",
        obj(&[("m02", Val::Float(20.0)), ("m12", Val::Float(20.0))]),
    );
    frame.data.set("strokePaints", paint(1.0, 0.0, 0.0));
    frame.data.set("strokeWeight", Val::Float(weight));
    frame.data.set("strokeAlign", Val::Str(align.into()));
    for (k, v) in fields {
        frame.data.set(k, v.clone());
    }
    frame
}

fn render(frame: figstudio_fig::Node) -> Vec<u8> {
    pixels(&document(vec![node(1, 0, "CANVAS"), frame]))
}

fn red(px: &[u8], x: usize, y: usize) -> bool {
    pixel(px, x, y) == [255, 0, 0, 255]
}

fn text(s: &str) -> Val {
    Val::Str(s.into())
}

fn dashed(cap: &str) -> Vec<u8> {
    let dash = Val::Arr(vec![Val::Float(4.0), Val::Float(4.0)]);
    render(frame(
        2.0,
        "CENTER",
        &[("dashPattern", dash), ("strokeCap", text(cap))],
    ))
}

/// Red pixels on the top edge's centre row, away from the corners.
fn top_edge_red(px: &[u8]) -> usize {
    (24..56).filter(|x| red(px, *x, 20)).count()
}

#[test]
fn miter_joins_keep_the_outer_corner_square() {
    let px = render(frame(8.0, "CENTER", &[("strokeJoin", text("MITER"))]));

    assert!(red(&px, 16, 16), "corner filled out to the miter point");
}

#[test]
fn round_and_bevel_joins_cut_the_outer_corner() {
    for join in ["ROUND", "BEVEL"] {
        let px = render(frame(8.0, "CENTER", &[("strokeJoin", text(join))]));

        assert!(!red(&px, 16, 16), "{join}: corner cut");
        assert!(red(&px, 16, 40), "{join}: edge still stroked");
    }
}

#[test]
fn a_dash_pattern_leaves_gaps_along_the_edge() {
    let px = dashed("NONE");

    let on = top_edge_red(&px);
    assert!(
        on > 8 && on < 24,
        "about half the edge is dashed, got {on}/32"
    );
}

#[test]
fn square_caps_lengthen_every_dash() {
    let butt = top_edge_red(&dashed("NONE"));
    let square = top_edge_red(&dashed("SQUARE"));

    assert!(square > butt, "square {square} vs butt {butt}");
}

fn sides(top: f64, right: Option<f64>, bottom: f64, left: f64) -> Vec<(&'static str, Val)> {
    let mut f = vec![
        ("borderStrokeWeightsIndependent", Val::Bool(true)),
        ("borderTopWeight", Val::Float(top)),
        ("borderBottomWeight", Val::Float(bottom)),
        ("borderLeftWeight", Val::Float(left)),
    ];
    // Figma omits a side whose weight is zero.
    f.extend(right.map(|r| ("borderRightWeight", Val::Float(r))));
    f
}

#[test]
fn independent_border_weights_draw_each_side_at_its_own_weight() {
    let px = render(frame(1.0, "INSIDE", &sides(4.0, None, 1.0, 2.0)));

    assert!(red(&px, 40, 20) && red(&px, 40, 23), "top 4 units");
    assert!(!red(&px, 40, 24), "top no deeper");
    assert!(
        red(&px, 20, 40) && red(&px, 21, 40) && !red(&px, 22, 40),
        "left 2"
    );
    assert!(red(&px, 40, 59) && !red(&px, 40, 58), "bottom 1");
    assert!(!red(&px, 59, 40), "no right side");
}

#[test]
fn outside_independent_border_weights_grow_outward() {
    let px = render(frame(1.0, "OUTSIDE", &sides(3.0, Some(0.0), 0.0, 0.0)));

    assert!(red(&px, 40, 17) && red(&px, 40, 19) && !red(&px, 40, 16));
    assert!(!red(&px, 40, 20), "nothing inside");
    assert!(!red(&px, 60, 40) && !red(&px, 40, 60), "zero sides");
}

fn rect_cmds(out: &mut Vec<u8>, x0: f32, y0: f32, x1: f32, y1: f32) {
    for (cmd, x, y) in [(1, x0, y0), (2, x1, y0), (2, x1, y1), (2, x0, y1)] {
        out.push(cmd);
        out.extend(x.to_le_bytes());
        out.extend(y.to_le_bytes());
    }
    out.push(0);
}

/// Component 3 holds frame 2 whose stored geometry is a 2-unit band on all four sides;
/// instance 10 overrides the frame's border weights to a top-only border.
fn overridden(extra: &[(&str, Val)]) -> Vec<u8> {
    let mut doc: Document = document(vec![]);
    let band = doc.blobs.len() as i64;
    let mut b = Vec::new();
    rect_cmds(&mut b, -1.0, -1.0, 41.0, 41.0);
    rect_cmds(&mut b, 1.0, 1.0, 39.0, 39.0);
    doc.blobs.push(b);
    let geom = Val::Arr(vec![obj(&[
        ("commandsBlob", Val::Int(band)),
        ("windingRule", text("ODD")),
    ])]);
    let child = framed(3, 2.0, "CENTER", &[("strokeGeometry", geom)]);
    let mut fields = sides(2.0, None, 0.0, 0.0);
    fields.extend(extra.iter().cloned());
    let mut inst = instance(10, 1, 3, vec![patch(&[2], &fields)]);
    inst.data.set("transform", obj(&[]));
    doc.set_nodes(vec![
        node(1, 0, "CANVAS"),
        node(3, 0, "SYMBOL"),
        child,
        inst,
    ])
    .unwrap();
    pixels(&doc)
}

#[test]
fn a_border_weight_override_replaces_stale_stroke_geometry() {
    let px = overridden(&[]);

    assert!(red(&px, 40, 20), "overridden top border");
    assert!(!red(&px, 20, 40), "stored left side is stale");
}

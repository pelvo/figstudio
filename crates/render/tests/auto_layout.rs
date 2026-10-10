//! Auto layout inside instances: constraints do not move in-flow children of a resized stack.
mod common;
use common::*;
use figstudio_fig::{Node, Val};

fn at(n: &mut Node, x: f64, y: f64, w: f64, h: f64) {
    n.data.set("size", size(w, h));
    n.data.set(
        "transform",
        obj(&[("m02", Val::Float(x)), ("m12", Val::Float(y))]),
    );
}

#[test]
fn scale_constraints_do_not_move_children_of_a_resized_stack_frame() {
    // A vertical stack frame (padding 6) holds a SCALE-constrained box; the instance shrinks
    // the frame from 40 to 20 wide. Auto layout keeps the box at the padding, x = 6.
    let mut frame = node(3, 2, "FRAME");
    at(&mut frame, 0.0, 0.0, 40.0, 20.0);
    frame.data.set("stackMode", Val::Str("VERTICAL".into()));
    frame.data.set("stackHorizontalPadding", Val::Float(6.0));
    let mut b = node(4, 3, "RECTANGLE");
    at(&mut b, 6.0, 0.0, 8.0, 20.0);
    b.data.set("fillPaints", paint(0.0, 1.0, 0.0));
    b.data.set("horizontalConstraint", Val::Str("SCALE".into()));
    // An absolute-positioned child still follows its MAX constraint: 30 -> 10.
    let mut abs = node(6, 3, "RECTANGLE");
    at(&mut abs, 30.0, 0.0, 4.0, 4.0);
    abs.data.set("fillPaints", paint(0.0, 0.0, 1.0));
    abs.data.set("horizontalConstraint", Val::Str("MAX".into()));
    abs.data
        .set("stackPositioning", Val::Str("ABSOLUTE".into()));
    let mut top = instance(5, 1, 2, vec![]);
    top.data.set(
        "derivedSymbolData",
        Val::Arr(vec![patch(&[3], &[("size", size(20.0, 20.0))])]),
    );
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        frame,
        b,
        abs,
        top,
    ]);
    let px = pixels(&doc);
    assert_eq!(pixel(&px, 7, 10), &[0, 255, 0, 255]);
    assert_eq!(pixel(&px, 13, 10), &[0, 255, 0, 255]);
    assert_eq!(pixel(&px, 4, 10), &[0, 0, 0, 0]);
    assert_eq!(pixel(&px, 12, 2), &[0, 0, 255, 255]);
    assert_eq!(pixel(&px, 32, 2), &[0, 0, 0, 0]);
}

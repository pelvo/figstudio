//! A frame that clips its content draws its stroke above its children; one that does not
//! clip draws it below them (KNOWN-GAPS A16: a table's outer border hidden by its cells).
mod common;
use common::*;
use figstudio_fig::{Document, Val};

/// A 40x40 frame at (20,20) with a 2-unit black inside stroke, fully covered by a red child.
fn framed(clips: bool) -> Document {
    let mut frame = node(2, 1, "FRAME");
    frame.data.set(
        "transform",
        obj(&[("m02", Val::Float(20.0)), ("m12", Val::Float(20.0))]),
    );
    frame.data.set("strokePaints", paint(0.0, 0.0, 0.0));
    frame.data.set("strokeWeight", Val::Float(2.0));
    frame.data.set("strokeAlign", Val::Str("INSIDE".into()));
    frame.data.set("frameMaskDisabled", Val::Bool(!clips));
    let mut cell = node(3, 2, "RECTANGLE");
    cell.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    document(vec![node(1, 0, "CANVAS"), frame, cell])
}

#[test]
fn clipping_frame_draws_its_stroke_above_its_children() {
    let px = pixels(&framed(true));
    // Every side of the inside stroke shows over the child.
    for (x, y) in [(20, 40), (59, 40), (40, 20), (40, 59)] {
        assert_eq!(pixel(&px, x, y), [0, 0, 0, 255], "edge at ({x},{y})");
    }
    assert_eq!(pixel(&px, 40, 40), [255, 0, 0, 255]);
}

#[test]
fn non_clipping_frame_draws_its_stroke_below_its_children() {
    let px = pixels(&framed(false));
    for (x, y) in [(20, 40), (59, 40), (40, 20), (40, 59)] {
        assert_eq!(pixel(&px, x, y), [255, 0, 0, 255], "edge at ({x},{y})");
    }
}

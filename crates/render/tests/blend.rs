//! Layer and fill blend modes mix with what is drawn below; modes Vello lacks are counted.
mod common;
use common::*;
use figstudio_fig::{Document, Node, Val};
use figstudio_render::{Renderer, Stats, Viewport};
use vello_cpu::kurbo::Affine;

/// A grey 40x40 square at (0,0) under a red 40x40 square at (20,20).
fn doc(node_mode: Option<&str>, paint_mode: Option<&str>) -> Document {
    let mut under = node(2, 1, "RECTANGLE");
    under.data.set("fillPaints", paint(0.5, 0.5, 0.5));
    let mut over = node(3, 1, "RECTANGLE");
    over.data.set(
        "transform",
        obj(&[("m02", Val::Float(20.0)), ("m12", Val::Float(20.0))]),
    );
    let mut fill = paint(1.0, 0.0, 0.0);
    if let (Some(mode), Val::Arr(paints)) = (paint_mode, &mut fill) {
        paints[0].set("blendMode", Val::Str(mode.into()));
    }
    over.data.set("fillPaints", fill);
    if let Some(mode) = node_mode {
        over.data.set("blendMode", Val::Str(mode.into()));
    }
    document(vec![node(1, 0, "CANVAS"), under, over])
}

fn render(doc: &Document) -> (Vec<u8>, Stats) {
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    let frame = renderer.render(
        doc,
        id(1),
        &Viewport {
            width: 80,
            height: 80,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    (frame.pixmap.data_as_u8_slice().to_vec(), frame.stats)
}

/// The overlap, the red square alone and the grey square alone.
fn samples(px: &[u8]) -> [[u8; 4]; 3] {
    [(30, 30), (50, 50), (10, 10)].map(|(x, y)| pixel(px, x, y).try_into().unwrap())
}

#[test]
fn layer_and_fill_multiply_darken_the_overlap_only() {
    for (node_mode, paint_mode) in [(Some("MULTIPLY"), None), (None, Some("MULTIPLY"))] {
        let (px, stats) = render(&doc(node_mode, paint_mode));
        let [overlap, red, grey] = samples(&px);
        assert!(
            overlap[0].abs_diff(128) <= 1 && overlap[1] == 0 && overlap[3] == 255,
            "grey x red = dark red, got {overlap:?} ({node_mode:?}, {paint_mode:?})"
        );
        assert_eq!(red, [255, 0, 0, 255], "over transparent the colour is kept");
        assert_eq!(grey[..3], [128, 128, 128]);
        assert_eq!(stats.approximated_blends, 0);
    }
}

#[test]
fn clipping_a_frame_does_not_isolate_its_children() {
    let mut d = doc(Some("MULTIPLY"), None);
    let mut nodes = d.nodes.clone();
    let mut frame = node(4, 1, "FRAME");
    frame.data.set("frameMaskDisabled", Val::Bool(false));
    frame.data.set("size", size(55.0, 55.0));
    for n in &mut nodes {
        if n.id == id(3) {
            let mut data = n.data.clone();
            data.set(
                "parentIndex",
                obj(&[("guid", id(4).to_val()), ("position", Val::Str("a".into()))]),
            );
            *n = Node::from_data(data).unwrap();
        }
    }
    nodes.push(frame);
    d.set_nodes(nodes).unwrap();
    let [overlap, red, _] = samples(&render(&d).0);
    assert!(
        overlap[0].abs_diff(128) <= 1 && overlap[1] == 0,
        "{overlap:?}"
    );
    assert_eq!(red, [255, 0, 0, 255]);
    assert_eq!(
        pixel(&render(&d).0, 58, 58)[3],
        0,
        "still clipped to the frame"
    );
}

#[test]
fn linear_dodge_adds_the_colours() {
    for (node_mode, paint_mode) in [(Some("LINEAR_DODGE"), None), (None, Some("LINEAR_DODGE"))] {
        let (px, stats) = render(&doc(node_mode, paint_mode));
        let [overlap, ..] = samples(&px);
        assert_eq!(overlap[0], 255, "{overlap:?}");
        assert!(overlap[1].abs_diff(128) <= 1, "{overlap:?}");
        assert_eq!(stats.approximated_blends, 0);
    }
}

#[test]
fn normal_and_pass_through_are_exact_and_unknown_modes_are_counted() {
    let (normal, stats) = render(&doc(None, None));
    assert_eq!(samples(&normal)[0], [255, 0, 0, 255]);
    assert_eq!(stats.approximated_blends, 0);
    for exact in ["NORMAL", "PASS_THROUGH"] {
        let (px, stats) = render(&doc(Some(exact), Some("NORMAL")));
        assert_eq!(px, normal, "{exact}");
        assert_eq!(stats.approximated_blends, 0, "{exact}");
    }
    // Vello has no linear burn: drawn as normal, and counted once per node and per paint.
    let (px, stats) = render(&doc(Some("LINEAR_BURN"), Some("LINEAR_BURN")));
    assert_eq!(px, normal);
    assert_eq!(stats.approximated_blends, 2);
}

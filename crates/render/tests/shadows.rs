//! Every visible drop shadow is drawn behind the node; inner shadows stay inside the shape.
mod common;
use common::*;
use figstudio_fig::{Document, Val};
use figstudio_render::{Renderer, Viewport};
use vello_cpu::kurbo::Affine;

fn shadow(kind: &str, dx: f64, dy: f64, radius: f64) -> Val {
    obj(&[
        ("type", Val::Str(kind.into())),
        ("visible", Val::Bool(true)),
        ("blendMode", Val::Str("NORMAL".into())),
        (
            "color",
            obj(&[
                ("r", Val::Float(0.0)),
                ("g", Val::Float(0.0)),
                ("b", Val::Float(0.0)),
                ("a", Val::Float(1.0)),
            ]),
        ),
        (
            "offset",
            obj(&[("x", Val::Float(dx)), ("y", Val::Float(dy))]),
        ),
        ("radius", Val::Float(radius)),
        ("spread", Val::Float(0.0)),
    ])
}

/// A white 40x40 rectangle at (20,20) with the given effects.
fn with_effects(effects: Vec<Val>) -> Document {
    let mut rect = node(2, 1, "RECTANGLE");
    rect.data.set(
        "transform",
        obj(&[("m02", Val::Float(20.0)), ("m12", Val::Float(20.0))]),
    );
    rect.data.set("fillPaints", paint(1.0, 1.0, 1.0));
    rect.data.set("effects", Val::Arr(effects));
    document(vec![node(1, 0, "CANVAS"), rect])
}

fn render(doc: &Document) -> (Vec<u8>, figstudio_render::Stats) {
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

#[test]
fn two_drop_shadows_in_different_directions_are_both_drawn() {
    let doc = with_effects(vec![
        shadow("DROP_SHADOW", 10.0, 0.0, 0.0),
        shadow("DROP_SHADOW", -10.0, 0.0, 0.0),
    ]);
    let (px, stats) = render(&doc);
    assert_eq!(pixel(&px, 65, 40), [0, 0, 0, 255], "right shadow");
    assert_eq!(pixel(&px, 15, 40), [0, 0, 0, 255], "left shadow");
    assert_eq!(
        pixel(&px, 40, 40),
        [255, 255, 255, 255],
        "node stays on top"
    );
    assert_eq!((stats.shadows, stats.skipped_effects), (2, 0));
}

#[test]
fn inner_shadow_darkens_the_inside_edge_only() {
    let doc = with_effects(vec![shadow("INNER_SHADOW", 6.0, 0.0, 0.0)]);
    let (px, stats) = render(&doc);
    // Offset right: the shadow falls on the left inside edge.
    assert_eq!(pixel(&px, 23, 40), [0, 0, 0, 255], "inside the left edge");
    assert_eq!(pixel(&px, 40, 40), [255, 255, 255, 255], "centre untouched");
    assert_eq!(pixel(&px, 15, 40)[3], 0, "nothing outside the shape");
    assert_eq!(pixel(&px, 65, 40)[3], 0, "nothing outside the shape");
    assert_eq!((stats.inner_shadows, stats.skipped_effects), (1, 0));
}

#[test]
fn unapplied_blend_mode_is_counted() {
    let mut multiply = shadow("DROP_SHADOW", 10.0, 0.0, 0.0);
    multiply.set("blendMode", Val::Str("MULTIPLY".into()));
    let (_, stats) = render(&with_effects(vec![multiply]));
    assert_eq!(stats.approximated_effects, 1);
}

//! Missing vector fills must be drawn or reported, not confused with stroke-only vectors.
mod common;
use common::*;
use figstudio_fig::Val;
use figstudio_render::{Renderer, Viewport};
use vello_cpu::kurbo::Affine;

#[test]
fn only_visible_vector_fills_without_paths_are_counted() {
    for geometry in [None, Some(Val::Arr(vec![]))] {
        for (fills, skipped) in [
            (None, 0),
            (Some(Val::Arr(vec![])), 0),
            (Some(paint(1.0, 0.0, 0.0)), 1),
            (Some(hidden_paint("visible", Val::Bool(false))), 0),
            (Some(hidden_paint("opacity", Val::Float(0.0))), 0),
        ] {
            let mut vector = node(2, 1, "VECTOR");
            if let Some(fills) = fills {
                vector.data.set("fillPaints", fills);
            }
            if let Some(geometry) = &geometry {
                vector.data.set("fillGeometry", geometry.clone());
            }
            vector.data.set("strokePaints", paint(0.0, 0.0, 0.0));
            let doc = document(vec![node(1, 0, "CANVAS"), vector]);
            let rendered = Renderer::new().render(&doc, id(1), &viewport());
            assert_eq!(rendered.stats.unsupported_paints, skipped);
        }
    }
}

fn hidden_paint(key: &str, value: Val) -> Val {
    let mut paints = paint(1.0, 0.0, 0.0).items().to_vec();
    paints[0].set(key, value);
    Val::Arr(paints)
}

fn viewport() -> Viewport {
    Viewport {
        width: 80,
        height: 80,
        transform: Affine::IDENTITY,
        background: None,
    }
}

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
fn overlapping_drop_shadows_follow_list_order() {
    let mut red = shadow("DROP_SHADOW", 10.0, 0.0, 0.0);
    red.set(
        "color",
        obj(&[("r", Val::Float(1.0)), ("a", Val::Float(1.0))]),
    );
    let mut blue = shadow("DROP_SHADOW", 12.0, 0.0, 0.0);
    blue.set(
        "color",
        obj(&[("b", Val::Float(1.0)), ("a", Val::Float(1.0))]),
    );
    let doc = with_effects(vec![red.clone(), blue.clone()]);
    let (px, stats) = render(&doc);
    assert_eq!(
        pixel(&px, 65, 40),
        [0, 0, 255, 255],
        "later blue covers red"
    );
    let (reversed, _) = render(&with_effects(vec![blue, red]));
    assert_eq!(
        pixel(&reversed, 65, 40),
        [255, 0, 0, 255],
        "reversing order exposes red"
    );
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

fn compound(effects: Vec<Val>, rule: &str, overlap: bool) -> Document {
    let mut doc = with_effects(effects);
    let mut commands = Vec::new();
    for (x0, y0, x1, y1) in [
        (0.0_f32, 0.0_f32, 40.0_f32, 40.0_f32),
        if overlap {
            (10.0, 0.0, 30.0, 40.0)
        } else {
            (10.0, 10.0, 30.0, 30.0)
        },
    ] {
        for (cmd, x, y) in [(1, x0, y0), (2, x1, y0), (2, x1, y1), (2, x0, y1)] {
            commands.push(cmd);
            commands.extend(x.to_le_bytes());
            commands.extend(y.to_le_bytes());
        }
        commands.push(0);
    }
    let blob = doc.blobs.len() as i64;
    doc.blobs.push(commands);
    let mut nodes = doc.nodes.clone();
    nodes[1].data.set(
        "fillGeometry",
        Val::Arr(vec![obj(&[
            ("commandsBlob", Val::Int(blob)),
            ("windingRule", Val::Str(rule.into())),
        ])]),
    );
    doc.set_nodes(nodes).unwrap();
    doc
}

#[test]
fn each_inner_shadow_installs_the_compound_shapes_clip_rule() {
    let mut transparent = shadow("INNER_SHADOW", 0.0, 0.0, 0.0);
    transparent.set("color", obj(&[("a", Val::Float(0.0))]));
    let doc = compound(
        vec![transparent, shadow("INNER_SHADOW", 50.0, 0.0, 0.0)],
        "NONZERO",
        false,
    );
    let (px, stats) = render(&doc);
    assert_eq!(
        pixel(&px, 40, 40),
        [0, 0, 0, 255],
        "second clip includes nested nonzero contour"
    );
    assert_eq!(stats.inner_shadows, 2);
}

#[test]
fn overlapping_nonzero_inner_mask_is_reported_as_approximate() {
    let doc = compound(vec![shadow("INNER_SHADOW", 0.0, 0.0, 0.0)], "NONZERO", true);
    let (_, stats) = render(&doc);
    assert_eq!((stats.inner_shadows, stats.approximated_effects), (1, 1));
}

#[test]
fn drop_mask_keeps_each_paths_own_rule() {
    let mut doc = compound(vec![shadow("DROP_SHADOW", 10.0, 0.0, 0.0)], "ODD", false);
    let mut nodes = doc.nodes.clone();
    let first = nodes[1].data.get("fillGeometry").unwrap().items()[0].clone();
    let mut second = first.clone();
    second.set("windingRule", Val::Str("NONZERO".into()));
    nodes[1]
        .data
        .set("fillGeometry", Val::Arr(vec![first, second]));
    doc.set_nodes(nodes).unwrap();
    let (px, stats) = render(&doc);
    assert_eq!(
        pixel(&px, 65, 40),
        [0, 0, 0, 255],
        "union must not cancel identical outer contours"
    );
    assert_eq!(stats.approximated_effects, 0);
}

#[test]
fn transparent_fills_do_not_cast_opaque_shadows() {
    for field in ["visible", "opacity", "alpha"] {
        let mut doc = with_effects(vec![shadow("DROP_SHADOW", 10.0, 0.0, 0.0)]);
        let mut nodes = doc.nodes.clone();
        let mut fill = nodes[1].data.get("fillPaints").unwrap().items()[0].clone();
        match field {
            "visible" => fill.set("visible", Val::Bool(false)),
            "opacity" => fill.set("opacity", Val::Float(0.0)),
            _ => {
                let mut color = fill.get("color").unwrap().clone();
                color.set("a", Val::Float(0.0));
                fill.set("color", color);
            }
        }
        nodes[1].data.set("fillPaints", Val::Arr(vec![fill]));
        doc.set_nodes(nodes).unwrap();
        let (px, _) = render(&doc);
        assert_eq!(pixel(&px, 65, 40)[3], 0, "{field}: no shadow without alpha");
    }
}

#[test]
fn complex_fill_silhouettes_cast_from_the_whole_node() {
    for case in [
        "translucent",
        "gradient",
        "image",
        "stroke",
        "child",
        "style",
    ] {
        let mut doc = with_effects(vec![shadow("DROP_SHADOW", 10.0, 0.0, 0.0)]);
        let mut nodes = doc.nodes.clone();
        let mut fill = nodes[1].data.get("fillPaints").unwrap().items()[0].clone();
        match case {
            "translucent" => fill.set("opacity", Val::Float(0.5)),
            "gradient" => fill.set("type", Val::Str("GRADIENT_LINEAR".into())),
            "image" => fill.set("type", Val::Str("IMAGE".into())),
            "stroke" => {
                nodes[1].data.set("strokePaints", paint(1.0, 0.0, 0.0));
                nodes[1].data.set("strokeWeight", Val::Float(8.0));
            }
            "child" => {
                nodes[1].kind = "FRAME".into();
                let mut child = node(3, 2, "RECTANGLE");
                child.data.set("fillPaints", paint(1.0, 1.0, 1.0));
                child
                    .data
                    .set("transform", obj(&[("m02", Val::Float(30.0))]));
                nodes.push(child);
            }
            _ => nodes[1].data.set(
                "vectorData",
                obj(&[(
                    "styleOverrideTable",
                    Val::Arr(vec![obj(&[("styleID", Val::Int(1))])]),
                )]),
            ),
        }
        nodes[1].data.set("fillPaints", Val::Arr(vec![fill]));
        doc.set_nodes(nodes).unwrap();
        let (_, stats) = render(&doc);
        assert_eq!(
            stats.approximated_effects, 0,
            "{case}: whole-node shadows are exact for fills, strokes and children"
        );
    }
}

#[test]
fn fallback_counts_unsupported_properties_and_each_skipped_shadow() {
    for stroke_only in [true, false] {
        let mut first = shadow("DROP_SHADOW", 10.0, 0.0, 0.0);
        first.set("blendMode", Val::Str("MULTIPLY".into()));
        first.set("spread", Val::Float(3.0));
        let mut doc = with_effects(vec![
            first,
            shadow("DROP_SHADOW", -10.0, 0.0, 0.0),
            shadow("INNER_SHADOW", 1.0, 0.0, 0.0),
        ]);
        let mut nodes = doc.nodes.clone();
        nodes[1].data.set("fillPaints", Val::Arr(vec![]));
        if stroke_only {
            nodes[1].data.set("strokePaints", paint(1.0, 1.0, 1.0));
            nodes[1].data.set("strokeWeight", Val::Float(4.0));
        } else {
            nodes[1].kind = "GROUP".into();
            let mut child = node(3, 2, "RECTANGLE");
            child.data.set("fillPaints", paint(1.0, 1.0, 1.0));
            nodes.push(child);
        }
        doc.set_nodes(nodes).unwrap();
        let (px, stats) = render(&doc);
        assert_eq!(
            (
                stats.shadows,
                stats.skipped_effects,
                stats.approximated_effects
            ),
            (1, 2, 1)
        );
        assert_eq!(pixel(&px, 69, 40)[3], 255, "fallback still draws a shadow");
    }
}

#[test]
fn many_shadows_are_bounded_and_each_omission_is_counted() {
    let effects = (0..64)
        .map(|i| {
            shadow(
                if i % 2 == 0 {
                    "DROP_SHADOW"
                } else {
                    "INNER_SHADOW"
                },
                2.0,
                0.0,
                2.0,
            )
        })
        .collect();
    let (px, stats) = render(&with_effects(effects));
    assert_eq!(
        (stats.shadows, stats.inner_shadows, stats.skipped_effects),
        (4, 4, 56)
    );
    let (_, baseline) = render(&with_effects(vec![]));
    assert_eq!(
        stats.attempted_work,
        baseline.attempted_work + 64,
        "each shadow uses the traversal budget"
    );
    assert!(pixel(&px, 61, 40)[3] > 0, "admitted shadows still render");
}

#[test]
fn anisotropic_blur_is_counted_for_fill_and_fallback_shadows() {
    for filled in [true, false] {
        for scale in [1.0, 0.5] {
            let mut doc = with_effects(vec![shadow("DROP_SHADOW", 4.0, 0.0, 6.0)]);
            let mut nodes = doc.nodes.clone();
            nodes[1].data.set(
                "transform",
                obj(&[
                    ("m00", Val::Float(scale)),
                    ("m02", Val::Float(20.0)),
                    ("m12", Val::Float(20.0)),
                ]),
            );
            if !filled {
                nodes[1].data.set("fillPaints", Val::Arr(vec![]));
                nodes[1].data.set("strokePaints", paint(1.0, 1.0, 1.0));
                nodes[1].data.set("strokeWeight", Val::Float(2.0));
            }
            doc.set_nodes(nodes).unwrap();
            let (_, stats) = render(&doc);
            assert_eq!(stats.approximated_effects, usize::from(scale != 1.0));
        }
    }
}

#[test]
fn blur_softens_the_shadow_edge_and_spread_expands_it() {
    let (hard, _) = render(&with_effects(vec![shadow("DROP_SHADOW", 10.0, 0.0, 0.0)]));
    let (blurred, _) = render(&with_effects(vec![shadow("DROP_SHADOW", 10.0, 0.0, 8.0)]));
    assert_eq!(pixel(&hard, 72, 40)[3], 0);
    let alpha = pixel(&blurred, 72, 40)[3];
    assert!(
        alpha > 0 && alpha < 255,
        "blur extends with partial alpha: {alpha}"
    );
    let mut spread = shadow("DROP_SHADOW", 10.0, 0.0, 0.0);
    spread.set("spread", Val::Float(4.0));
    let (expanded, _) = render(&with_effects(vec![spread]));
    assert_eq!(pixel(&expanded, 72, 40)[3], 255);
}

#[test]
fn one_effect_with_multiple_limitations_is_counted_once() {
    let mut effect = shadow("INNER_SHADOW", 2.0, 0.0, 4.0);
    effect.set("blendMode", Val::Str("MULTIPLY".into()));
    effect.set("spread", Val::Float(-2.0));
    let mut doc = compound(vec![effect], "NONZERO", true);
    let mut nodes = doc.nodes.clone();
    nodes[1]
        .data
        .set("transform", obj(&[("m00", Val::Float(0.5))]));
    doc.set_nodes(nodes).unwrap();
    let (_, stats) = render(&doc);
    assert_eq!(stats.approximated_effects, 1);
}

#[test]
fn unapplied_blend_mode_is_counted() {
    let mut multiply = shadow("DROP_SHADOW", 10.0, 0.0, 0.0);
    multiply.set("blendMode", Val::Str("MULTIPLY".into()));
    let (_, stats) = render(&with_effects(vec![multiply]));
    assert_eq!(stats.approximated_effects, 1);
}

/// A frame without a fill, a white 10x10 child at (10,10), and the given effects on the frame.
fn frame_with_child(effects: Vec<Val>) -> Document {
    let mut frame = node(2, 1, "FRAME");
    frame.data.set("fillPaints", Val::Arr(vec![]));
    frame.data.set("effects", Val::Arr(effects));
    let mut child = node(3, 2, "RECTANGLE");
    child.data.set("fillPaints", paint(1.0, 1.0, 1.0));
    child.data.set("size", size(10.0, 10.0));
    child.data.set(
        "transform",
        obj(&[("m02", Val::Float(10.0)), ("m12", Val::Float(10.0))]),
    );
    document(vec![node(1, 0, "CANVAS"), frame, child])
}

fn coloured(mut s: Val, rgb: (f64, f64, f64)) -> Val {
    s.set(
        "color",
        obj(&[
            ("r", Val::Float(rgb.0)),
            ("g", Val::Float(rgb.1)),
            ("b", Val::Float(rgb.2)),
            ("a", Val::Float(1.0)),
        ]),
    );
    s
}

#[test]
fn shadow_of_a_fill_less_frame_follows_its_child() {
    let (px, stats) = render(&frame_with_child(vec![shadow(
        "DROP_SHADOW",
        10.0,
        0.0,
        0.0,
    )]));
    assert_eq!(pixel(&px, 25, 15)[3], 255, "shadow follows the child");
    assert_eq!(pixel(&px, 35, 15)[3], 0, "nothing follows the frame bounds");
    assert_eq!((stats.approximated_effects, stats.shadows), (0, 1));
}

#[test]
fn stroke_alpha_casts_shadow_outside_the_fill() {
    let mut doc = with_effects(vec![shadow("DROP_SHADOW", 10.0, 0.0, 0.0)]);
    let mut nodes = doc.nodes.clone();
    nodes[1].data.set("strokePaints", paint(0.0, 1.0, 0.0));
    nodes[1].data.set("strokeWeight", Val::Float(12.0));
    doc.set_nodes(nodes).unwrap();
    let (px, stats) = render(&doc);
    // Fill ends at x=60; a centred 12px stroke reaches x=66, so the shadow reaches x=76.
    assert_eq!(pixel(&px, 74, 40)[3], 255, "shadow follows the stroke");
    assert_eq!(stats.approximated_effects, 0);
}

#[test]
fn two_shadows_with_children_are_not_cast_from_each_other() {
    let red = coloured(shadow("DROP_SHADOW", -10.0, 0.0, 0.0), (1.0, 0.0, 0.0));
    let blue = coloured(shadow("DROP_SHADOW", 10.0, 0.0, 0.0), (0.0, 0.0, 1.0));
    let (px, stats) = render(&frame_with_child(vec![red, blue]));
    assert_eq!(pixel(&px, 5, 15), [255, 0, 0, 255], "first shadow, behind");
    assert_eq!(pixel(&px, 25, 15), [0, 0, 255, 255], "second shadow");
    assert_eq!(pixel(&px, 15, 15), [255, 255, 255, 255], "content on top");
    // A shadow of the blue shadow would put red at x 25..35 and blue at 35..45.
    assert_eq!(pixel(&px, 35, 15)[3], 0, "no shadow of a shadow");
    assert_eq!(pixel(&px, 45, 15)[3], 0, "no shadow of a shadow");
    assert_eq!(
        (
            stats.shadows,
            stats.skipped_effects,
            stats.approximated_effects
        ),
        (2, 0, 0)
    );
    assert_eq!(stats.nodes, 2, "replays are not counted as drawn nodes");
}

#[test]
fn translucent_fill_casts_a_translucent_shadow() {
    let mut doc = with_effects(vec![shadow("DROP_SHADOW", 10.0, 0.0, 0.0)]);
    let mut nodes = doc.nodes.clone();
    let mut fill = nodes[1].data.get("fillPaints").unwrap().items()[0].clone();
    fill.set("opacity", Val::Float(0.5));
    nodes[1].data.set("fillPaints", Val::Arr(vec![fill]));
    doc.set_nodes(nodes).unwrap();
    let (px, stats) = render(&doc);
    let alpha = pixel(&px, 65, 40)[3];
    assert!(
        (126..=130).contains(&alpha),
        "shadow alpha follows source alpha: {alpha}"
    );
    assert_eq!(stats.approximated_effects, 0);
}

#[test]
fn faded_node_fades_its_whole_node_shadow() {
    let mut doc = frame_with_child(vec![shadow("DROP_SHADOW", 10.0, 0.0, 0.0)]);
    let mut nodes = doc.nodes.clone();
    nodes[1].data.set("opacity", Val::Float(0.5));
    doc.set_nodes(nodes).unwrap();
    let alpha = pixel(&render(&doc).0, 25, 15)[3];
    assert!((126..=130).contains(&alpha), "{alpha}");
}

#[test]
fn spread_stays_on_the_counted_mask_path() {
    let mut spread = shadow("DROP_SHADOW", 0.0, 0.0, 0.0);
    spread.set("spread", Val::Float(5.0));
    let (px, stats) = render(&with_effects(vec![spread]));
    assert_eq!(pixel(&px, 17, 40)[3], 255, "spread grows the shadow");
    assert_eq!(
        stats.approximated_effects, 0,
        "an opaque rectangle with spread is exact on the mask path"
    );
    // A fill-less frame cannot use the mask path: spread is counted, not silently dropped.
    let mut spread = shadow("DROP_SHADOW", 0.0, 0.0, 0.0);
    spread.set("spread", Val::Float(5.0));
    let (_, stats) = render(&frame_with_child(vec![spread]));
    assert_eq!(stats.approximated_effects, 1);
}

#[test]
fn plain_opaque_rectangle_keeps_the_fill_mask_path() {
    // Blur 0 and an opaque fill: the mask path is exact and nothing is replayed or layered.
    let (px, stats) = render(&with_effects(vec![shadow("DROP_SHADOW", 10.0, 0.0, 0.0)]));
    assert_eq!(pixel(&px, 65, 40), [0, 0, 0, 255]);
    assert_eq!(pixel(&px, 45, 40), [255, 255, 255, 255]);
    assert_eq!(
        (
            stats.shadows,
            stats.approximated_effects,
            stats.skipped_effects
        ),
        (1, 0, 0)
    );
}

#[test]
fn oversized_layers_fall_back_counted_without_panic() {
    // A viewport larger than the layer budget: the old path takes over and counts the effect.
    let mut doc = frame_with_child(vec![shadow("DROP_SHADOW", 10.0, 0.0, 0.0)]);
    let mut nodes = doc.nodes.clone();
    nodes[1].data.set("fillPaints", paint(0.5, 0.5, 0.5));
    nodes[1].data.set("size", size(4096.0, 2048.0));
    doc.set_nodes(nodes).unwrap();
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    let frame = renderer.render(
        &doc,
        id(1),
        &Viewport {
            width: 4096,
            height: 2048,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    assert_eq!(frame.stats.approximated_effects, 1);
    assert_eq!(frame.stats.shadows, 1);
}

#[test]
fn many_children_with_two_shadows_fall_back_counted() {
    let red = coloured(shadow("DROP_SHADOW", -10.0, 0.0, 0.0), (1.0, 0.0, 0.0));
    let blue = coloured(shadow("DROP_SHADOW", 10.0, 0.0, 0.0), (0.0, 0.0, 1.0));
    let mut doc = frame_with_child(vec![red, blue]);
    let mut nodes = doc.nodes.clone();
    nodes[1].data.set("fillPaints", paint(0.5, 0.5, 0.5));
    for n in 4..4 + 600 {
        let mut c = node(n, 2, "RECTANGLE");
        c.data.set("size", size(1.0, 1.0));
        nodes.push(c);
    }
    doc.set_nodes(nodes).unwrap();
    let (_, stats) = render(&doc);
    assert_eq!(
        stats.approximated_effects, 2,
        "both shadows use the counted path"
    );
}

#[test]
fn content_is_composited_once_however_many_shadows() {
    // Shadows stay on the canvas: vello_cpu clips a layer to the canvas before it shifts it.
    let red = coloured(shadow("DROP_SHADOW", -20.0, 0.0, 0.0), (1.0, 0.0, 0.0));
    let blue = coloured(shadow("DROP_SHADOW", 20.0, 0.0, 0.0), (0.0, 0.0, 1.0));
    let mut doc = frame_with_child(vec![red, blue]);
    let mut nodes = doc.nodes.clone();
    nodes[2].data.set(
        "transform",
        obj(&[("m02", Val::Float(30.0)), ("m12", Val::Float(10.0))]),
    );
    let mut fill = nodes[2].data.get("fillPaints").unwrap().items()[0].clone();
    fill.set("opacity", Val::Float(0.5));
    nodes[2].data.set("fillPaints", Val::Arr(vec![fill]));
    doc.set_nodes(nodes).unwrap();
    let alpha = pixel(&render(&doc).0, 35, 15)[3];
    assert!(
        (126..=130).contains(&alpha),
        "a translucent child drawn once: {alpha}"
    );
}

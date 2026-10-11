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

fn render_sized(doc: &Document, width: u16, height: u16) -> figstudio_render::Stats {
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    renderer
        .render(
            doc,
            id(1),
            &Viewport {
                width,
                height,
                transform: Affine::IDENTITY,
                background: None,
            },
        )
        .stats
}

/// A 1x1 opaque frame with a big child: the node bounds are tiny, the rendered alpha is not.
fn tiny_frame_with_big_child(clip: bool) -> Document {
    let mut frame = node(2, 1, "FRAME");
    frame.data.set("size", size(1.0, 1.0));
    frame.data.set("fillPaints", paint(0.5, 0.5, 0.5));
    frame.data.set("frameMaskDisabled", Val::Bool(!clip));
    frame.data.set(
        "effects",
        Val::Arr(vec![shadow("DROP_SHADOW", 1.0, 0.0, 0.0)]),
    );
    let mut child = node(3, 2, "RECTANGLE");
    child.data.set("size", size(4200.0, 2000.0));
    child.data.set("fillPaints", paint(1.0, 1.0, 1.0));
    document(vec![node(1, 0, "CANVAS"), frame, child])
}

#[test]
fn guard_measures_the_rendered_alpha_not_the_node_box() {
    let stats = render_sized(&tiny_frame_with_big_child(false), 4200, 2000);
    assert_eq!(stats.approximated_effects, 1, "8.4M px layer is refused");
}

#[test]
fn guard_ignores_children_clipped_by_the_frame() {
    let stats = render_sized(&tiny_frame_with_big_child(true), 4200, 2000);
    assert_eq!(stats.approximated_effects, 0, "clipped: the layer is tiny");
}

#[test]
fn mixed_whole_and_mask_shadows_keep_list_order() {
    let red = coloured(shadow("DROP_SHADOW", 10.0, 0.0, 0.0), (1.0, 0.0, 0.0));
    let mut blue = coloured(shadow("DROP_SHADOW", 10.0, 0.0, 0.0), (0.0, 0.0, 1.0));
    blue.set("spread", Val::Float(3.0));
    let mut doc = with_effects(vec![red, blue]);
    let mut nodes = doc.nodes.clone();
    nodes[1].data.set("strokePaints", paint(1.0, 1.0, 1.0));
    nodes[1].data.set("strokeWeight", Val::Float(2.0));
    doc.set_nodes(nodes).unwrap();
    let (px, _) = render(&doc);
    assert_eq!(
        pixel(&px, 70, 40),
        [0, 0, 255, 255],
        "later blue covers red"
    );
}

#[test]
fn inner_shadows_do_not_change_the_replayed_shadow_alpha() {
    // A half-transparent fill whose inner shadow makes it opaque; a second, invisible drop
    // shadow must not change the first one.
    let build = |extra: bool| {
        let mut inner = shadow("INNER_SHADOW", 100.0, 0.0, 0.0);
        inner.set("spread", Val::Float(0.0));
        let mut effects = vec![shadow("DROP_SHADOW", 20.0, 0.0, 0.0)];
        if extra {
            let mut clear = shadow("DROP_SHADOW", 30.0, 0.0, 0.0);
            clear.set(
                "color",
                obj(&[("r", Val::Float(0.0)), ("a", Val::Float(0.0))]),
            );
            effects.push(clear);
        }
        effects.push(inner);
        let mut doc = with_effects(effects);
        let mut nodes = doc.nodes.clone();
        let mut fill = nodes[1].data.get("fillPaints").unwrap().items()[0].clone();
        fill.set("opacity", Val::Float(0.5));
        nodes[1].data.set("fillPaints", Val::Arr(vec![fill]));
        doc.set_nodes(nodes).unwrap();
        pixel(&render(&doc).0, 70, 40)[3]
    };
    assert_eq!(build(true), build(false));
    assert_eq!(build(false), 255, "the inner shadow makes the node opaque");
}

fn fill_less_frame(n: u32, parent: u32, effects: Vec<Val>) -> figstudio_fig::Node {
    let mut f = node(n, parent, "FRAME");
    f.data.set("fillPaints", Val::Arr(vec![]));
    f.data.set("effects", Val::Arr(effects));
    f
}

#[test]
fn nested_multi_shadow_children_keep_both_shadows_inside_a_parent_shadow() {
    let child_effects = vec![
        coloured(shadow("DROP_SHADOW", 20.0, 0.0, 0.0), (0.0, 1.0, 0.0)),
        coloured(shadow("DROP_SHADOW", 40.0, 0.0, 0.0), (0.0, 0.0, 1.0)),
    ];
    let parent_effects = vec![
        coloured(shadow("DROP_SHADOW", 0.0, 20.0, 0.0), (1.0, 0.0, 0.0)),
        coloured(shadow("DROP_SHADOW", 0.0, 60.0, 0.0), (1.0, 1.0, 0.0)),
    ];
    let mut g = node(4, 3, "RECTANGLE");
    g.data.set("fillPaints", paint(1.0, 1.0, 1.0));
    g.data.set("size", size(10.0, 10.0));
    g.data.set(
        "transform",
        obj(&[("m02", Val::Float(10.0)), ("m12", Val::Float(10.0))]),
    );
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        fill_less_frame(2, 1, parent_effects),
        fill_less_frame(3, 2, child_effects),
        g,
    ]);
    let (px, stats) = render(&doc);
    assert_eq!(
        pixel(&px, 55, 15),
        [0, 0, 255, 255],
        "child's own second shadow"
    );
    assert_eq!(
        pixel(&px, 55, 35),
        [255, 0, 0, 255],
        "the parent's shadow includes the child's second shadow"
    );
    assert_eq!(stats.approximated_effects, 0);
}

#[test]
fn refused_effects_are_not_drawn() {
    // Eight inner shadows use the per-node cap; the ninth effect (a drop shadow) is refused.
    let mut effects = vec![shadow("INNER_SHADOW", 1.0, 0.0, 0.0); 8];
    effects.push(shadow("DROP_SHADOW", 10.0, 0.0, 0.0));
    let mut doc = with_effects(effects);
    let mut nodes = doc.nodes.clone();
    nodes[1].data.set("strokePaints", paint(1.0, 1.0, 1.0));
    nodes[1].data.set("strokeWeight", Val::Float(2.0));
    doc.set_nodes(nodes).unwrap();
    let (px, stats) = render(&doc);
    assert_eq!(stats.skipped_effects, 1);
    assert_eq!(stats.shadows, 0, "no drop shadow is drawn");
    assert_eq!(pixel(&px, 67, 40)[3], 0, "and none is visible");
}

#[test]
fn faded_node_fades_every_shadow_of_several() {
    let red = coloured(shadow("DROP_SHADOW", -20.0, 0.0, 0.0), (1.0, 0.0, 0.0));
    let blue = coloured(shadow("DROP_SHADOW", 20.0, 0.0, 0.0), (0.0, 0.0, 1.0));
    let mut doc = frame_with_child(vec![red, blue]);
    let mut nodes = doc.nodes.clone();
    nodes[1].data.set("opacity", Val::Float(0.5));
    nodes[2].data.set(
        "transform",
        obj(&[("m02", Val::Float(30.0)), ("m12", Val::Float(10.0))]),
    );
    doc.set_nodes(nodes).unwrap();
    let px = render(&doc).0;
    for x in [15, 55] {
        let a = pixel(&px, x, 15)[3];
        assert!((126..=130).contains(&a), "shadow at x={x} is faded: {a}");
    }
}

/// Render size where a whole-content layer would pass the 8M-pixel guard if the guard were
/// fooled: the target is large, so only the bounds decide.
const BIG: (u16, u16) = (4200, 2000);

fn effect_dx(dx: f64) -> Val {
    shadow("DROP_SHADOW", dx, 0.0, 0.0)
}

#[test]
fn fill_less_shadow_fallback_obeys_the_size_guard() {
    // A fill-less 1x1 frame with a huge child: no mask fallback exists, so the refused
    // shadow is skipped and counted instead of becoming another whole-content layer.
    let mut doc = tiny_frame_with_big_child(false);
    let mut nodes = doc.nodes.clone();
    nodes[1].data.set("fillPaints", Val::Arr(vec![]));
    doc.set_nodes(nodes).unwrap();
    let stats = render_sized(&doc, BIG.0, BIG.1);
    assert_eq!(stats.shadows, 0, "no layer is drawn");
    assert_eq!(stats.skipped_effects, 1, "and the omission is counted");
}

#[test]
fn clipping_does_not_trim_the_nodes_own_stroke() {
    let mut frame = node(2, 1, "FRAME");
    frame.data.set("size", size(1000.0, 1000.0));
    frame.data.set("frameMaskDisabled", Val::Bool(false));
    frame.data.set("fillPaints", paint(0.5, 0.5, 0.5));
    frame.data.set("strokePaints", paint(1.0, 0.0, 0.0));
    frame.data.set("strokeWeight", Val::Float(1200.0));
    frame.data.set("effects", Val::Arr(vec![effect_dx(1.0)]));
    let doc = document(vec![node(1, 0, "CANVAS"), frame]);
    let stats = render_sized(&doc, 4200, 2000);
    assert_eq!(
        stats.approximated_effects, 1,
        "the stroke makes a >8M px layer"
    );
}

/// A symbol with one 10x10 child, and an instance whose override changes that child.
fn instance_with_override(fields: &[(&str, Val)]) -> Document {
    let mut symbol = node(2, 0, "SYMBOL");
    symbol.data.set("size", size(10.0, 10.0));
    let mut child = node(3, 2, "RECTANGLE");
    child.data.set("size", size(10.0, 10.0));
    child.data.set("fillPaints", paint(1.0, 1.0, 1.0));
    let mut inst = instance(4, 1, 2, vec![patch(&[3], fields)]);
    inst.data.set("size", size(10.0, 10.0));
    inst.data.set("fillPaints", paint(0.5, 0.5, 0.5));
    inst.data.set("effects", Val::Arr(vec![effect_dx(1.0)]));
    document(vec![node(1, 0, "CANVAS"), symbol, child, inst])
}

#[test]
fn instance_override_that_enlarges_a_child_is_guarded() {
    let big = instance_with_override(&[("size", size(4200.0, 2000.0))]);
    let stats = render_sized(&big, 4200, 2000);
    assert_eq!(
        stats.approximated_effects, 1,
        "uncertain bounds: whole target"
    );
}

#[test]
fn border_weight_override_cannot_enlarge_a_layer_past_the_guard() {
    // Per-side border weights were missing from the list of ink-changing override keys.
    let fields = [
        ("borderBottomWeight", Val::Float(8400.0)),
        ("borderStrokeWeightsIndependent", Val::Bool(true)),
    ];
    let doc = instance_with_override(&fields);
    let stats = render_sized(&doc, 4200, 2000);
    assert_eq!(stats.approximated_effects, 1);
}

#[test]
fn any_instance_uses_the_whole_target_as_its_bound() {
    // On a target under the cap everything fits; over it, an instance is refused whatever its
    // overrides are (a paint-only override included: no list of keys to keep in sync).
    let same = instance_with_override(&[("fillPaints", paint(0.0, 1.0, 0.0))]);
    assert_eq!(render_sized(&same, 4200, 2000).approximated_effects, 1);
    assert_eq!(render_sized(&same, 80, 80).approximated_effects, 0);
}

#[test]
fn symbol_swap_cannot_enlarge_a_layer_past_the_guard() {
    // A plain frame (no clip) holds an instance of a tiny symbol; an override on the outer
    // instance swaps the inner one for a symbol with a huge child. Bounds read from the
    // stored (tiny) symbol would admit the layer.
    let mut tiny = node(4, 0, "SYMBOL");
    tiny.data.set("size", size(10.0, 10.0));
    let mut big = node(6, 0, "SYMBOL");
    big.data.set("size", size(10.0, 10.0));
    let mut huge = node(7, 6, "RECTANGLE");
    huge.data.set("size", size(4200.0, 2000.0));
    huge.data.set("fillPaints", paint(1.0, 1.0, 1.0));
    let mut holder = node(2, 0, "SYMBOL");
    holder.data.set("size", size(10.0, 10.0));
    holder.data.set("frameMaskDisabled", Val::Bool(true));
    let swap = patch(&[3], &[("overriddenSymbolID", id(6).to_val())]);
    let mut frame = node(20, 1, "FRAME");
    frame.data.set("size", size(10.0, 10.0));
    frame.data.set("frameMaskDisabled", Val::Bool(true));
    frame.data.set("fillPaints", paint(0.5, 0.5, 0.5));
    frame.data.set("effects", Val::Arr(vec![effect_dx(1.0)]));
    let mut top = instance(10, 20, 2, vec![swap]);
    top.data.set("size", size(10.0, 10.0));
    let mut placeholder = instance(3, 2, 4, vec![]);
    placeholder.data.set("size", size(10.0, 10.0));
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        holder,
        placeholder,
        tiny,
        big,
        huge,
        frame,
        top,
    ]);
    assert_eq!(render_sized(&doc, 4200, 2000).approximated_effects, 1);
}

/// A frame holding an instance whose override swaps a tiny symbol for one with 5000 children
/// (10x10 white squares at (40,0)); the frame has two drop shadows that move down 40 and 50.
fn swapped_frame(fill: bool) -> Document {
    let mut nodes = vec![node(1, 0, "CANVAS"), node(2, 0, "SYMBOL")];
    nodes.push(instance(3, 2, 4, vec![]));
    nodes.push(node(4, 0, "SYMBOL"));
    nodes.push(node(6, 0, "SYMBOL"));
    for n in 0..5000 {
        let mut c = node(100 + n, 6, "RECTANGLE");
        c.data.set("size", size(10.0, 10.0));
        c.data.set("fillPaints", paint(1.0, 1.0, 1.0));
        c.data.set("transform", obj(&[("m02", Val::Float(40.0))]));
        nodes.push(c);
    }
    let mut frame = node(20, 1, "FRAME");
    frame.data.set("size", size(10.0, 10.0));
    frame.data.set(
        "fillPaints",
        if fill {
            paint(0.5, 0.5, 0.5)
        } else {
            Val::Arr(vec![])
        },
    );
    let red = coloured(shadow("DROP_SHADOW", 0.0, 40.0, 0.0), (1.0, 0.0, 0.0));
    let blue = coloured(shadow("DROP_SHADOW", 0.0, 50.0, 0.0), (0.0, 0.0, 1.0));
    frame.data.set("effects", Val::Arr(vec![red, blue]));
    let swap = patch(&[3], &[("overriddenSymbolID", id(6).to_val())]);
    nodes.push(frame);
    nodes.push(instance(10, 20, 2, vec![swap]));
    document(nodes)
}

#[test]
fn replay_work_is_enforced_inside_the_traversal() {
    let mut plain = swapped_frame(true);
    let mut nodes = plain.nodes.clone();
    nodes
        .iter_mut()
        .find(|n| n.id == id(20))
        .unwrap()
        .data
        .set("effects", Val::Arr(vec![]));
    plain.set_nodes(nodes).unwrap();
    let base = render(&plain).1.attempted_work;
    let (_, stats) = render(&swapped_frame(true));
    assert!(
        stats.attempted_work < base + 500,
        "replays stop at their allowance: {} vs {base} without shadows",
        stats.attempted_work
    );
    assert_eq!(
        stats.approximated_effects, 2,
        "both shadows fell back, counted"
    );
}

#[test]
fn an_exhausted_replay_leaves_no_half_shadow() {
    let (px, _) = render(&swapped_frame(true));
    // The first children's shadows (y 40..50 and 50..60 at x 40..50) would show here if the
    // partial replay layer were composited.
    assert_eq!(pixel(&px, 45, 45)[3], 0);
    assert_eq!(pixel(&px, 45, 55)[3], 0);
}

#[test]
fn fill_less_frame_skips_a_refused_replay_instead_of_casting_its_box() {
    let (px, stats) = render(&swapped_frame(false));
    assert_eq!(
        stats.skipped_effects, 2,
        "no fill mask: skipped and counted"
    );
    assert_eq!(pixel(&px, 5, 45)[3], 0, "the frame box casts nothing");
}

/// `levels` nested 3000x2000 frames (each layer 6M px: under the per-layer cap) with a shadow.
fn nested_near_cap_layers(levels: u32) -> figstudio_render::Stats {
    let mut nodes = vec![node(1, 0, "CANVAS")];
    for i in 0..levels {
        let mut f = node(2 + i, if i == 0 { 1 } else { 1 + i }, "FRAME");
        f.data.set("size", size(3000.0, 2000.0));
        f.data.set("fillPaints", paint(0.5, 0.5, 0.5));
        f.data.set("effects", Val::Arr(vec![effect_dx(1.0)]));
        nodes.push(f);
    }
    let mut leaf = node(100, 1 + levels, "RECTANGLE");
    leaf.data.set("size", size(5.0, 5.0));
    nodes.push(leaf);
    render_sized(&document(nodes), 3000, 2000)
}

#[test]
fn nested_near_cap_layers_stop_at_the_aggregate_budget() {
    // Each layer is under the per-layer cap; together they are not: 64M px of budget fits ten
    // 6M px layers, the other two fall back to the counted fill mask.
    let stats = nested_near_cap_layers(12);
    assert_eq!(
        stats.approximated_effects, 2,
        "ten layers fit, two fall back"
    );
}

#[test]
fn pushed_layer_pixels_are_reported_and_within_the_total_budget() {
    let stats = nested_near_cap_layers(12);
    assert!(stats.shadow_layer_pixels > 0);
    // MAX_SHADOW_LAYER_PIXELS_TOTAL
    assert!(
        stats.shadow_layer_pixels <= 64_000_000,
        "{}",
        stats.shadow_layer_pixels
    );
}

/// `levels` nested frames in a symbol; an override on the instance gives each two shadows.
fn nested_override_shadows(levels: u32) -> usize {
    let mut nodes = vec![node(1, 0, "CANVAS"), node(2, 0, "SYMBOL")];
    let mut patches = Vec::new();
    for i in 0..levels {
        let n = 3 + i;
        let parent = if i == 0 { 2 } else { n - 1 };
        let mut f = node(n, parent, "FRAME");
        f.data.set("size", size(10.0, 10.0));
        f.data.set("fillPaints", paint(1.0, 1.0, 1.0));
        nodes.push(f);
        // Override paths name instances, not the ordinary frames above the node.
        let effects = Val::Arr(vec![effect_dx(1.0), effect_dx(2.0)]);
        patches.push(patch(&[n], &[("effects", effects)]));
    }
    let mut inst = instance(100, 1, 2, patches);
    inst.data.set("size", size(10.0, 10.0));
    nodes.push(inst);
    render(&document(nodes)).1.attempted_work
}

#[test]
fn override_added_shadows_cannot_blow_up_the_work() {
    // A static estimate over stored effects sees no shadows here; unbounded redraws would
    // multiply by 3 per level (2,000,000 work units, the whole budget, at 20 levels).
    let (twenty, forty) = (nested_override_shadows(20), nested_override_shadows(40));
    assert!(twenty < 10_000, "20 levels: {twenty} work units");
    assert!(forty < 8 * twenty, "at most quadratic: {forty} vs {twenty}");
}

#[test]
fn plain_opaque_rectangle_stays_on_the_fill_mask_path_pixel_for_pixel() {
    // Fractional placement and offset, translucent shadow: the mask path gives 152 here, a
    // whole-node layer (blurred-alpha path, rounded differently) gives 153.
    let mut rect = node(2, 1, "RECTANGLE");
    rect.data.set(
        "transform",
        obj(&[("m02", Val::Float(20.3)), ("m12", Val::Float(20.3))]),
    );
    rect.data.set("fillPaints", paint(1.0, 1.0, 1.0));
    let mut s = shadow("DROP_SHADOW", 7.3, 0.0, 0.0);
    s.set(
        "color",
        obj(&[
            ("r", Val::Float(0.0)),
            ("g", Val::Float(0.0)),
            ("b", Val::Float(0.0)),
            ("a", Val::Float(0.7)),
        ]),
    );
    rect.data.set("effects", Val::Arr(vec![s]));
    let (px, stats) = render(&document(vec![node(1, 0, "CANVAS"), rect]));
    assert_eq!(pixel(&px, 60, 20), [54, 54, 54, 152]);
    assert_eq!(stats.approximated_effects, 0);
}

#[cfg(feature = "parallel")]
#[test]
fn a_discarded_redraw_is_erased_in_parallel_strips_too() {
    let doc = swapped_frame(true);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let viewport = Viewport {
        width: 80,
        height: 128,
        transform: Affine::IDENTITY,
        background: None,
    };
    let mut r = Renderer::new();
    r.system_fonts = false;
    let one = r.render(&doc, id(1), &viewport);
    let many = pool.install(|| r.render_parallel(&doc, id(1), &viewport));
    assert_eq!(one.stats, many.stats);
    let (a, b) = (
        one.pixmap.data_as_u8_slice(),
        many.pixmap.data_as_u8_slice(),
    );
    let alpha = |px: &[u8], x: usize, y: usize| px[(y * 80 + x) * 4 + 3];
    for (x, y) in [(45, 45), (45, 55), (45, 100)] {
        assert_eq!(alpha(a, x, y), 0, "single: no half shadow at ({x},{y})");
        assert_eq!(alpha(b, x, y), 0, "strips: no half shadow at ({x},{y})");
    }
}

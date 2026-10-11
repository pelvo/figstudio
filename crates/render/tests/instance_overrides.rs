//! Synthetic override path and geometry regressions.
mod common;
use common::*;
use figstudio_fig::Val;

#[test]
fn uniform_instance_scale_updates_omitted_translation_but_not_precomputed_geometry() {
    let mut rect = node(4, 3, "RECTANGLE");
    rect.data.set("size", size(10.0, 10.0));
    rect.data.set("fillPaints", paint(0.0, 1.0, 0.0));
    rect.data.set(
        "transform",
        obj(&[
            ("m00", Val::Float(-1.0)),
            ("m02", Val::Float(15.0)),
            ("m12", Val::Float(5.0)),
        ]),
    );
    let mut top = instance(5, 1, 2, vec![]);
    top.data
        .get_mut("symbolData")
        .unwrap()
        .set("uniformScaleFactor", Val::Float(2.0));
    let mut doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        node(3, 2, "FRAME"),
        rect,
        top.clone(),
    ]);
    let blob = doc.blobs.len() as i64;
    let mut commands = Vec::new();
    for (cmd, x, y) in [
        (1, 0f32, 0f32),
        (2, 10.0, 0.0),
        (2, 10.0, 20.0),
        (2, 0.0, 20.0),
    ] {
        commands.push(cmd);
        commands.extend(x.to_le_bytes());
        commands.extend(y.to_le_bytes());
    }
    commands.push(0);
    doc.blobs.push(commands);
    top.data.set(
        "derivedSymbolData",
        Val::Arr(vec![patch(
            &[4],
            &[
                ("size", size(10.0, 20.0)),
                (
                    "fillGeometry",
                    Val::Arr(vec![obj(&[("commandsBlob", Val::Int(blob))])]),
                ),
            ],
        )]),
    );
    let mut nodes = doc.nodes.clone();
    *nodes.iter_mut().find(|n| n.id == id(5)).unwrap() = top;
    doc.set_nodes(nodes).unwrap();
    let px = pixels(&doc);
    assert_eq!(pixel(&px, 25, 25), &[0, 255, 0, 255]);
    assert_eq!(pixel(&px, 15, 15), &[0, 0, 0, 0]);
    assert_eq!(pixel(&px, 25, 35), &[0, 0, 0, 0]);
}

#[test]
fn override_paths_skip_frames_but_keep_instance_ancestors() {
    let mut rect = node(5, 4, "RECTANGLE");
    rect.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        node(3, 2, "FRAME"),
        instance(6, 3, 7, vec![]),
        node(7, 0, "SYMBOL"),
        node(4, 7, "FRAME"),
        rect,
        instance(
            8,
            1,
            2,
            vec![patch(&[6, 5], &[("fillPaints", paint(0.0, 1.0, 0.0))])],
        ),
    ]);
    assert_eq!(pixel(&pixels(&doc), 10, 10), &[0, 255, 0, 255]);
}

#[test]
fn resized_frames_move_centered_children_without_replacing_rotations_or_fresh_transforms() {
    let mut rect = node(4, 3, "RECTANGLE");
    rect.data.set("size", size(10.0, 10.0));
    rect.data.set("fillPaints", paint(0.0, 1.0, 0.0));
    rect.data
        .set("horizontalConstraint", Val::Str("CENTER".into()));
    rect.data
        .set("verticalConstraint", Val::Str("CENTER".into()));
    rect.data.set(
        "transform",
        obj(&[
            ("m00", Val::Float(0.0)),
            ("m01", Val::Float(-1.0)),
            ("m10", Val::Float(1.0)),
            ("m11", Val::Float(0.0)),
            ("m02", Val::Float(20.0)),
            ("m12", Val::Float(10.0)),
        ]),
    );
    let top = instance(5, 1, 2, vec![patch(&[3], &[("size", size(80.0, 80.0))])]);
    let mut doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        node(3, 2, "FRAME"),
        rect,
        top,
    ]);
    let px = pixels(&doc);
    assert_eq!(pixel(&px, 35, 35), &[0, 255, 0, 255]);
    assert_eq!(pixel(&px, 15, 15), &[0, 0, 0, 0]);
    let mut nodes = doc.nodes.clone();
    nodes.iter_mut().find(|n| n.id == id(5)).unwrap().data.set(
        "derivedSymbolData",
        Val::Arr(vec![patch(
            &[4],
            &[(
                "transform",
                obj(&[("m02", Val::Float(50.0)), ("m12", Val::Float(50.0))]),
            )],
        )]),
    );
    doc.set_nodes(nodes).unwrap();
    assert_eq!(pixel(&pixels(&doc), 55, 55), &[0, 255, 0, 255]);
}

fn scaled_instance(n: u32, parent: u32, symbol: u32, factor: f64) -> figstudio_fig::Node {
    let mut result = instance(n, parent, symbol, vec![]);
    result
        .data
        .get_mut("symbolData")
        .unwrap()
        .set("uniformScaleFactor", Val::Float(factor));
    result
}

fn at(x: f64, y: f64) -> Val {
    let mut result = obj(&[]);
    result.set("m02", Val::Float(x));
    result.set("m12", Val::Float(y));
    result
}

#[test]
fn nested_constraints_run_before_outer_scaling_without_overriding_stated_placement() {
    for (constraint, stack, absolute, explicit, outer_size, expected_x) in [
        ("CENTER", false, false, false, false, 30),
        ("MAX", false, false, false, false, 50),
        ("SCALE", false, false, false, false, 15),
        ("CENTER", true, false, false, false, 10),
        ("CENTER", true, true, false, false, 30),
        ("CENTER", false, false, true, false, 10),
        ("CENTER", false, false, false, true, 25),
    ] {
        let mut frame = node(5, 4, "FRAME");
        if stack {
            frame.data.set("stackMode", Val::Str("HORIZONTAL".into()));
        }
        let mut rect = node(6, 5, "RECTANGLE");
        rect.data.set("size", size(10.0, 10.0));
        rect.data.set("transform", at(5.0, 5.0));
        rect.data.set("fillPaints", paint(0.0, 1.0, 0.0));
        rect.data
            .set("horizontalConstraint", Val::Str(constraint.into()));
        if absolute {
            rect.data
                .set("stackPositioning", Val::Str("ABSOLUTE".into()));
        }
        let mut nested = scaled_instance(3, 2, 4, 1.0);
        let mut patches = vec![patch(&[5], &[("size", size(60.0, 40.0))])];
        if explicit {
            patches.push(patch(&[6], &[("transform", at(5.0, 5.0))]));
        }
        nested.data.set("derivedSymbolData", Val::Arr(patches));
        let mut top = scaled_instance(7, 1, 2, 2.0);
        if outer_size {
            top.data.set(
                "derivedSymbolData",
                Val::Arr(vec![patch(&[3, 6], &[("size", size(30.0, 20.0))])]),
            );
        }
        let doc = document(vec![
            node(1, 0, "CANVAS"),
            node(2, 0, "SYMBOL"),
            nested,
            node(4, 0, "SYMBOL"),
            frame,
            rect,
            top,
        ]);
        let px = pixels(&doc);
        assert_eq!(
            pixel(&px, expected_x + 1, 11),
            &[0, 255, 0, 255],
            "{constraint} stack={stack} explicit={explicit}"
        );
        let width = if outer_size { 30 } else { 20 };
        assert_eq!(pixel(&px, expected_x + width - 1, 29), &[0, 255, 0, 255]);
        assert_eq!(pixel(&px, expected_x + width + 1, 11), &[0, 0, 0, 0]);
        assert_eq!(pixel(&px, expected_x - 1, 11), &[0, 0, 0, 0]);
    }
}

#[test]
fn inherited_inside_stroke_scales_but_outer_stated_weight_does_not() {
    for (independent, outer_weight) in [(false, false), (true, false), (false, true), (true, true)]
    {
        let mut rect = node(5, 4, "RECTANGLE");
        rect.data.set("size", size(10.0, 10.0));
        rect.data.set("transform", at(5.0, 5.0));
        rect.data.set("strokePaints", paint(1.0, 0.0, 0.0));
        rect.data.set("strokeAlign", Val::Str("INSIDE".into()));
        rect.data.set("strokeWeight", Val::Float(2.0));
        rect.data
            .set("borderStrokeWeightsIndependent", Val::Bool(independent));
        for key in [
            "borderTopWeight",
            "borderRightWeight",
            "borderBottomWeight",
            "borderLeftWeight",
        ] {
            rect.data.set(key, Val::Float(2.0));
        }
        let mut nested = scaled_instance(3, 2, 4, 3.0);
        nested.data.set(
            "derivedSymbolData",
            Val::Arr(vec![patch(
                &[5],
                &[
                    ("size", size(10.0, 10.0)),
                    ("transform", at(5.0, 5.0)),
                    ("strokeWeight", Val::Float(2.0)),
                    ("borderTopWeight", Val::Float(2.0)),
                    ("borderRightWeight", Val::Float(2.0)),
                    ("borderBottomWeight", Val::Float(2.0)),
                    ("borderLeftWeight", Val::Float(2.0)),
                ],
            )]),
        );
        let mut top = scaled_instance(7, 1, 2, 2.0);
        let mut fields = vec![];
        if outer_weight {
            for key in [
                "strokeWeight",
                "borderTopWeight",
                "borderRightWeight",
                "borderBottomWeight",
                "borderLeftWeight",
            ] {
                fields.push((key, Val::Float(2.0)));
            }
        }
        top.data
            .set("derivedSymbolData", Val::Arr(vec![patch(&[3, 5], &fields)]));
        let doc = document(vec![
            node(1, 0, "CANVAS"),
            node(2, 0, "SYMBOL"),
            nested,
            node(4, 0, "SYMBOL"),
            rect,
            top,
        ]);
        let px = pixels(&doc);
        assert_eq!(pixel(&px, 11, 20), &[255, 0, 0, 255]);
        assert_eq!(
            pixel(&px, 13, 20),
            if outer_weight {
                &[0, 0, 0, 0]
            } else {
                &[255, 0, 0, 255]
            }
        );
        assert_eq!(pixel(&px, 9, 20), &[0, 0, 0, 0]);
    }
}

#[test]
fn inherited_glyph_em_size_and_position_scale_unless_outer_owner_states_them() {
    for outer_glyphs in [false, true] {
        let mut text = node(5, 4, "TEXT");
        text.data.set("fillPaints", paint(0.0, 0.0, 1.0));
        let mut doc = document(vec![
            node(1, 0, "CANVAS"),
            node(2, 0, "SYMBOL"),
            scaled_instance(3, 2, 4, 3.0),
            node(4, 0, "SYMBOL"),
            text,
            scaled_instance(7, 1, 2, 2.0),
        ]);
        let blob = doc.blobs.len() as i64;
        let mut commands = Vec::new();
        for (cmd, x, y) in [
            (1, 0f32, 0f32),
            (2, 1.0, 0.0),
            (2, 1.0, -1.0),
            (2, 0.0, -1.0),
        ] {
            commands.push(cmd);
            commands.extend(x.to_le_bytes());
            commands.extend(y.to_le_bytes());
        }
        commands.push(0);
        doc.blobs.push(commands);
        let glyphs = obj(&[(
            "glyphs",
            Val::Arr(vec![obj(&[
                ("commandsBlob", Val::Int(blob)),
                ("fontSize", Val::Float(10.0)),
                ("position", size(2.0, 2.0)),
            ])]),
        )]);
        let mut nodes = doc.nodes.clone();
        nodes.iter_mut().find(|n| n.id == id(3)).unwrap().data.set(
            "derivedSymbolData",
            Val::Arr(vec![patch(
                &[5],
                &[
                    ("size", size(14.0, 14.0)),
                    ("transform", at(5.0, 5.0)),
                    ("derivedTextData", glyphs.clone()),
                ],
            )]),
        );
        if outer_glyphs {
            nodes.iter_mut().find(|n| n.id == id(7)).unwrap().data.set(
                "derivedSymbolData",
                Val::Arr(vec![patch(&[3, 5], &[("derivedTextData", glyphs)])]),
            );
        }
        doc.set_nodes(nodes).unwrap();
        let px = pixels(&doc);
        assert_eq!(pixel(&px, 15, 15), &[0, 0, 255, 255]);
        assert_eq!(
            pixel(&px, 25, 25),
            if outer_glyphs {
                &[0, 0, 0, 0]
            } else {
                &[0, 0, 255, 255]
            }
        );
        assert_eq!(
            pixel(&px, 13, 13),
            if outer_glyphs {
                &[0, 0, 255, 255]
            } else {
                &[0, 0, 0, 0]
            }
        );
        assert_eq!(pixel(&px, 35, 35), &[0, 0, 0, 0]);
    }
}

#[test]
fn inherited_stored_geometry_approximation_is_visible_in_status() {
    for (outer_geometry, inherited_only, mask) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (false, true, true),
    ] {
        let mut rect = node(5, 4, "RECTANGLE");
        rect.data.set("fillPaints", paint(0.0, 1.0, 0.0));
        rect.data.set("mask", Val::Bool(mask));
        let mut doc = document(vec![
            node(1, 0, "CANVAS"),
            node(2, 0, "SYMBOL"),
            scaled_instance(3, 2, 4, 1.0),
            node(4, 0, "SYMBOL"),
            rect,
            scaled_instance(7, 1, 2, 2.0),
        ]);
        let blob = doc.blobs.len() as i64;
        let mut commands = Vec::new();
        for (cmd, x, y) in [
            (1, 0f32, 0f32),
            (2, 10.0, 0.0),
            (2, 10.0, 10.0),
            (2, 0.0, 10.0),
        ] {
            commands.push(cmd);
            commands.extend(x.to_le_bytes());
            commands.extend(y.to_le_bytes());
        }
        commands.push(0);
        doc.blobs.push(commands);
        let geometry = Val::Arr(vec![obj(&[("commandsBlob", Val::Int(blob))])]);
        let mut nodes = doc.nodes.clone();
        if inherited_only {
            nodes
                .iter_mut()
                .find(|n| n.id == id(5))
                .unwrap()
                .data
                .set("fillGeometry", geometry.clone());
        } else {
            nodes.iter_mut().find(|n| n.id == id(3)).unwrap().data.set(
                "derivedSymbolData",
                Val::Arr(vec![patch(
                    &[5],
                    &[
                        ("size", size(10.0, 10.0)),
                        ("fillGeometry", geometry.clone()),
                    ],
                )]),
            );
        }
        if outer_geometry {
            nodes.iter_mut().find(|n| n.id == id(7)).unwrap().data.set(
                "derivedSymbolData",
                Val::Arr(vec![patch(&[3, 5], &[("fillGeometry", geometry)])]),
            );
        }
        doc.set_nodes(nodes).unwrap();
        let mut renderer = figstudio_render::Renderer::new();
        renderer.system_fonts = false;
        let rendered = renderer.render(
            &doc,
            id(1),
            &figstudio_render::Viewport {
                width: 80,
                height: 80,
                transform: vello_cpu::kurbo::Affine::IDENTITY,
                background: None,
            },
        );
        assert_eq!(
            rendered.stats.approximated_instance_ink,
            usize::from(!outer_geometry)
        );
        assert_eq!(
            rendered
                .stats
                .to_string()
                .contains("instance ink approximated"),
            !outer_geometry
        );
    }
}

#[test]
fn mixed_override_sources_keep_each_fields_owner() {
    for derived in [false, true] {
        for outer_position in [Some(false), Some(true), None] {
            let mut rect = node(5, 4, "RECTANGLE");
            rect.data.set("fillPaints", paint(0.0, 1.0, 0.0));
            let mut nested = scaled_instance(3, 2, 4, 3.0);
            let inner = Val::Arr(vec![patch(
                &[5],
                &[("transform", at(4.0, 4.0)), ("size", size(4.0, 4.0))],
            )]);
            if derived {
                nested.data.set("derivedSymbolData", inner);
            } else {
                nested
                    .data
                    .get_mut("symbolData")
                    .unwrap()
                    .set("symbolOverrides", inner);
            }
            let mut top = scaled_instance(7, 1, 2, 2.0);
            // Both owners target the same child, but the winning fields come from different depths.
            let fields = match outer_position {
                Some(true) => vec![("transform", at(24.0, 24.0))],
                Some(false) => vec![("size", size(16.0, 16.0))],
                None => vec![("transform", at(32.0, 32.0)), ("size", size(12.0, 12.0))],
            };
            if derived {
                top.data
                    .get_mut("symbolData")
                    .unwrap()
                    .set("symbolOverrides", Val::Arr(vec![patch(&[3, 5], &fields)]));
            } else {
                top.data
                    .set("derivedSymbolData", Val::Arr(vec![patch(&[3, 5], &fields)]));
            }
            let doc = document(vec![
                node(1, 0, "CANVAS"),
                node(2, 0, "SYMBOL"),
                nested,
                node(4, 0, "SYMBOL"),
                rect,
                top,
            ]);
            let (start, width) = match outer_position {
                Some(true) => (24, 8),
                Some(false) => (8, 16),
                None => (32, 12),
            };
            let px = pixels(&doc);
            assert_eq!(pixel(&px, start + 1, start + 1), &[0, 255, 0, 255]);
            assert_eq!(
                pixel(&px, start + width - 1, start + width - 1),
                &[0, 255, 0, 255]
            );
            assert_eq!(
                pixel(&px, start + width + 1, start + width + 1),
                &[0, 0, 0, 0]
            );
        }
    }
}

#[test]
fn three_level_scaling_uses_only_factors_outside_the_field_owner() {
    let mut rect = node(7, 6, "RECTANGLE");
    rect.data.set("fillPaints", paint(0.0, 1.0, 0.0));
    let mut inner = scaled_instance(5, 4, 6, 4.0);
    inner.data.set(
        "derivedSymbolData",
        Val::Arr(vec![patch(
            &[7],
            &[("transform", at(4.0, 4.0)), ("size", size(4.0, 4.0))],
        )]),
    );
    let mut inherited = node(9, 6, "RECTANGLE");
    inherited.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    inherited.data.set("transform", at(1.0, 2.0));
    inherited.data.set("size", size(1.0, 1.0));
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        scaled_instance(3, 2, 4, 3.0),
        node(4, 0, "SYMBOL"),
        inner,
        node(6, 0, "SYMBOL"),
        rect,
        inherited,
        scaled_instance(8, 1, 2, 2.0),
    ]);
    let px = pixels(&doc);
    assert_eq!(pixel(&px, 25, 25), &[0, 255, 0, 255]);
    assert_eq!(pixel(&px, 47, 47), &[0, 255, 0, 255]);
    assert_eq!(pixel(&px, 23, 23), &[0, 0, 0, 0]);
    assert_eq!(pixel(&px, 49, 49), &[0, 0, 0, 0]);
    assert_eq!(pixel(&px, 25, 49), &[255, 0, 0, 255]);
    assert_eq!(pixel(&px, 47, 71), &[255, 0, 0, 255]);
    assert_eq!(pixel(&px, 25, 73), &[0, 0, 0, 0]);
}

#[test]
fn uniform_scale_applies_to_values_stated_by_nested_instances_but_not_by_itself() {
    let mut inner_rect = node(5, 4, "RECTANGLE");
    inner_rect.data.set("fillPaints", paint(0.0, 1.0, 0.0));
    let mut outer_rect = node(6, 4, "RECTANGLE");
    outer_rect.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    let at = |x: f64, y: f64| obj(&[("m02", Val::Float(x)), ("m12", Val::Float(y))]);
    let mut nested = instance(3, 2, 4, vec![]);
    // The nested instance restates its child's place and size in its own, unscaled space.
    nested.data.set(
        "derivedSymbolData",
        Val::Arr(vec![patch(
            &[5],
            &[("transform", at(5.0, 5.0)), ("size", size(10.0, 10.0))],
        )]),
    );
    let mut top = instance(7, 1, 2, vec![]);
    top.data
        .get_mut("symbolData")
        .unwrap()
        .set("uniformScaleFactor", Val::Float(2.0));
    // The scaled instance itself states the other child already in scaled space.
    top.data.set(
        "derivedSymbolData",
        Val::Arr(vec![patch(
            &[3, 6],
            &[("transform", at(50.0, 50.0)), ("size", size(10.0, 10.0))],
        )]),
    );
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        nested,
        node(4, 0, "SYMBOL"),
        inner_rect,
        outer_rect,
        top,
    ]);
    let px = pixels(&doc);
    assert_eq!(pixel(&px, 11, 11), &[0, 255, 0, 255]);
    assert_eq!(pixel(&px, 28, 28), &[0, 255, 0, 255]);
    assert_eq!(pixel(&px, 32, 32), &[0, 0, 0, 0]);
    assert_eq!(pixel(&px, 55, 55), &[255, 0, 0, 255]);
    assert_eq!(pixel(&px, 62, 62), &[0, 0, 0, 0]);
}

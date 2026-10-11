//! get_design_context regressions from the cross-family review of the first L6 commit.
#[path = "design/support.rs"]
mod support;
use figstudio_fig::Val;
use figstudio_mcp::tools::design::{MAX_CODE_BYTES, MAX_INLINE_BYTES, Options, design_context};
use support::*;

fn number(value: f64, units: &'static str) -> Val {
    obj(&[("value", f(value)), ("units", e(units))])
}
fn alias(id: u32) -> Val {
    obj(&[("value", obj(&[("alias", obj(&[("guid", g(id).to_val())]))]))])
}
fn color_variable(id: u32, name: &str, color: Val) -> figstudio_fig::Node {
    let entry = obj(&[(
        "variableData",
        obj(&[("value", obj(&[("colorValue", color)]))]),
    )]);
    let mut v = node(
        id,
        PAGE,
        "VARIABLE",
        name,
        &[(
            "variableDataValues",
            obj(&[("entries", Val::Arr(vec![entry]))]),
        )],
    );
    v.parent = None;
    v
}

#[test]
fn the_size_cap_holds_for_long_names_and_deep_trees() {
    let long = node(1, PAGE, "FRAME", &"<".repeat(300_000), &[]);
    let mut deep = vec![node(2, PAGE, "FRAME", "d", &[])];
    deep.extend((3..4_000).map(|i| node(i, g(i - 1), "FRAME", "d", &[])));
    let mut nodes = vec![long];
    nodes.extend(deep);
    let doc = doc_with(nodes);
    for id in ["9:1", "9:2"] {
        let result = design_context(
            &doc,
            id,
            &Options {
                max_nodes: Some(5_000),
                ..inline()
            },
        );
        let text = texts(&result);
        assert!(
            text[0].len() <= MAX_CODE_BYTES,
            "{id}: {} bytes",
            text[0].len()
        );
        assert!(text[1].contains("TRUNCATED"), "{id}: {}", text[1]);
    }
    // The long-named root alone does not fit: the reply says so instead of breaking the cap.
    let text = texts(&design_context(
        &doc,
        "9:2",
        &Options {
            max_nodes: Some(5_000),
            ..inline()
        },
    ));
    let opened = text[0].matches("<div").count();
    assert_eq!(
        text[0].matches("</div>").count() + text[0].matches(" />").count(),
        opened,
        "balanced"
    );
}

/// A 512×512 PNG of noise, which compresses badly, so each asset render is large.
fn noise_png() -> Vec<u8> {
    let mut state = 1u32;
    let img = image::RgbaImage::from_fn(512, 512, |_, _| {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        image::Rgba(state.to_le_bytes())
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn inline_asset_bytes_are_bounded_and_omissions_reported() {
    let hash = [7u8; 20];
    let photo = obj(&[
        ("type", e("IMAGE")),
        ("image", obj(&[("hash", Val::Bytes(hash.to_vec()))])),
    ]);
    let shapes: Vec<_> = (1..=12)
        .map(|i| {
            node(
                i,
                PAGE,
                "RECTANGLE",
                "photo",
                &[
                    ("size", obj(&[("x", f(512.0)), ("y", f(512.0))])),
                    ("fillPaints", Val::Arr(vec![photo.clone()])),
                ],
            )
        })
        .collect();
    let mut doc = doc_with(shapes);
    doc.images.insert(figstudio_fig::hex(&hash), noise_png());
    let result = design_context(&doc, "0:1", &inline());
    let json = serde_json::to_value(&result).unwrap();
    let images: Vec<usize> = json["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["type"] == "image")
        .map(|c| c["data"].as_str().unwrap().len())
        .collect();
    assert!(
        !images.is_empty() && images.iter().sum::<usize>() <= MAX_INLINE_BYTES,
        "{images:?}"
    );
    let notes = &texts(&result)[1];
    assert!(
        notes.contains("assets not rendered (inline size cap)"),
        "{notes}"
    );
}

#[test]
fn an_instance_root_shows_its_component_fill_and_radius() {
    let symbol = node(
        1,
        PAGE,
        "SYMBOL",
        "component",
        &[
            (
                "fillPaints",
                Val::Arr(vec![solid(rgba(1.0, 0.0, 0.0, 1.0))]),
            ),
            ("cornerRadius", f(4.0)),
        ],
    );
    let child = node(2, g(1), "RECTANGLE", "child", &[]);
    let instance = node(
        3,
        PAGE,
        "INSTANCE",
        "instance",
        &[
            ("symbolData", obj(&[("symbolID", g(1).to_val())])),
            ("transform", at(50.0, 0.0)),
        ],
    );
    let out = code(&doc_with(vec![symbol, child, instance]), "9:3");
    assert!(
        out.starts_with(r#"<div data-node-id="9:3" data-name="instance""#),
        "{out}"
    );
    let first = out.lines().next().unwrap();
    assert!(
        first.contains("bg-[#FF0000]") && first.contains("rounded-[4px]"),
        "{first}"
    );
}

#[test]
fn arbitrary_values_have_no_spaces_and_bound_stops_resolve() {
    let stops = Val::Arr(vec![
        obj(&[("position", f(0.0)), ("color", rgba(1.0, 0.0, 0.0, 1.0))]),
        obj(&[("position", f(1.0)), ("color", rgba(0.0, 1.0, 0.0, 1.0))]),
    ]);
    let stops_var = Val::Arr(vec![
        obj(&[
            ("position", f(0.0)),
            ("color", rgba(1.0, 0.0, 0.0, 1.0)),
            ("colorVar", alias(20)),
        ]),
        obj(&[("position", f(1.0)), ("color", rgba(0.0, 1.0, 0.0, 1.0))]),
    ]);
    let radial = obj(&[
        ("type", e("GRADIENT_RADIAL")),
        ("stops", stops.clone()),
        ("stopsVar", stops_var),
    ]);
    let a = node(
        1,
        PAGE,
        "RECTANGLE",
        "radial",
        &[("fillPaints", Val::Arr(vec![radial]))],
    );
    let angular = obj(&[("type", e("GRADIENT_ANGULAR")), ("stops", stops)]);
    let b = node(
        2,
        PAGE,
        "RECTANGLE",
        "angular",
        &[("fillPaints", Val::Arr(vec![angular]))],
    );
    let doc = doc_with(vec![
        a,
        b,
        color_variable(20, "Brand", rgba(0.0, 0.0, 0.0, 1.0)),
    ]);
    for id in ["9:1", "9:2"] {
        let out = code(&doc, id);
        let class = out
            .split("className=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        for token in class.split(' ') {
            assert!(
                !token.contains('[') || token.ends_with(']'),
                "{id}: split token {token} in {class}"
            );
        }
    }
    let out = code(&doc, "9:1");
    assert!(out.contains("var(--brand,#000000)_0%"), "{out}");
}

#[test]
fn line_height_units_and_small_tracking() {
    let text = |id, lh: Val, ls: Val| {
        node(
            id,
            PAGE,
            "TEXT",
            "t",
            &[
                ("textData", obj(&[("characters", Val::Str("x".into()))])),
                ("fontSize", f(10.0)),
                ("lineHeight", lh),
                ("letterSpacing", ls),
                ("fontWeight", f(900.0)),
                (
                    "fontName",
                    obj(&[
                        ("family", Val::Str("Inter".into())),
                        ("style", Val::Str("Light".into())),
                    ]),
                ),
            ],
        )
    };
    let doc = doc_with(vec![
        text(1, number(1.5, "RAW"), number(0.5, "PIXELS")),
        text(2, number(18.0, "PIXELS"), number(2.0, "PERCENT")),
        text(3, number(0.0, "RAW"), number(0.0, "PIXELS")),
    ]);
    let one = code(&doc, "9:1");
    assert!(
        one.contains("leading-[1.5]") && one.contains("tracking-[0.5px]"),
        "{one}"
    );
    assert!(
        one.contains("font-[300]") && !one.contains("font-[900]"),
        "weight from fontName.style: {one}"
    );
    let two = code(&doc, "9:2");
    assert!(
        two.contains("leading-[18px]") && two.contains("tracking-[0.02em]"),
        "{two}"
    );
    assert!(!notes(&doc, "9:1").contains("approximations"));
    let three = code(&doc, "9:3");
    assert!(
        !three.contains("leading-") && !three.contains("tracking-"),
        "{three}"
    );
}

#[test]
fn asset_opacity_is_not_applied_twice() {
    let mut v = node(
        1,
        PAGE,
        "RECTANGLE",
        "v",
        &[
            (
                "fillPaints",
                Val::Arr(vec![solid(rgba(1.0, 0.0, 0.0, 1.0))]),
            ),
            ("opacity", f(0.5)),
            ("blendMode", e("MULTIPLY")),
        ],
    );
    v.kind = "VECTOR".into();
    v.data.set("type", e("VECTOR"));
    let out = code(&doc_with(vec![v]), "9:1");
    assert!(out.contains("<img") && !out.contains("opacity-"), "{out}");
    assert!(
        out.contains("mix-blend-multiply"),
        "blending with the backdrop is not in the PNG: {out}"
    );
}

#[test]
fn effect_styles_resolve() {
    let shadow = obj(&[
        ("type", e("DROP_SHADOW")),
        ("color", rgba(0.0, 0.0, 0.0, 1.0)),
        ("offset", obj(&[("x", f(1.0)), ("y", f(2.0))])),
        ("radius", f(3.0)),
    ]);
    let mut style = node(
        30,
        PAGE,
        "RECTANGLE",
        "Elevation",
        &[
            ("styleType", e("EFFECT")),
            ("effects", Val::Arr(vec![shadow])),
        ],
    );
    style.parent = None;
    let reference = obj(&[("guid", g(30).to_val())]);
    let a = node(
        1,
        PAGE,
        "RECTANGLE",
        "current",
        &[("styleIdForEffect", reference)],
    );
    let b = node(
        2,
        PAGE,
        "RECTANGLE",
        "legacy",
        &[("inheritEffectStyleID", g(30).to_val())],
    );
    let doc = doc_with(vec![style, a, b]);
    for id in ["9:1", "9:2"] {
        let out = code(&doc, id);
        assert!(
            out.contains("shadow-[1px_2px_3px_0px_#000000]"),
            "{id}: {out}"
        );
    }
}

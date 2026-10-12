//! get_design_context mappings on documents built in the test, and the structure.fig fixture.
#[path = "design/support.rs"]
mod support;
use figstudio_fig::Val;
use support::*;

#[test]
fn auto_layout_becomes_flex_with_gap_padding_and_alignment() {
    let frame = node(
        1,
        PAGE,
        "FRAME",
        "row",
        &[
            ("stackMode", e("HORIZONTAL")),
            ("stackSpacing", f(8.0)),
            ("stackCounterSpacing", f(f64::NAN)),
            ("stackHorizontalPadding", f(16.0)),
            ("stackVerticalPadding", f(4.0)),
            ("stackPaddingRight", f(12.0)),
            ("stackPrimaryAlignItems", e("SPACE_EVENLY")),
            ("stackCounterAlignItems", e("CENTER")),
            ("stackPrimarySizing", e("RESIZE_TO_FIT")),
        ],
    );
    let grow = node(
        2,
        g(1),
        "FRAME",
        "grow",
        &[("stackChildPrimaryGrow", f(1.0))],
    );
    let stretch = node(
        3,
        g(1),
        "FRAME",
        "stretch",
        &[("stackChildAlignSelf", e("STRETCH"))],
    );
    let pinned = node(
        4,
        g(1),
        "FRAME",
        "pinned",
        &[
            ("stackPositioning", e("ABSOLUTE")),
            ("transform", at(3.0, 5.0)),
        ],
    );
    let out = code(&doc_with(vec![frame, grow, stretch, pinned]), "9:1");
    let want = [
        r#"<div data-node-id="9:1" data-name="row" className="h-[10px] flex relative gap-[8px] pt-[4px] pr-[12px] pb-[4px] pl-[16px] justify-evenly items-center overflow-clip">"#,
        r#"  <div data-node-id="9:2" data-name="grow" className="grow shrink-0 h-[10px]" />"#,
        r#"  <div data-node-id="9:3" data-name="stretch" className="self-stretch shrink-0 w-[10px]" />"#,
        r#"  <div data-node-id="9:4" data-name="pinned" className="absolute left-[3px] top-[5px] w-[10px] h-[10px]" />"#,
        "</div>",
    ];
    assert_eq!(out.lines().collect::<Vec<_>>(), want, "{out}");
}

#[test]
fn absolute_children_rotation_and_wrapped_stacks() {
    let mut rotated = at(4.0, 6.0);
    rotated.set("m00", f(0.0));
    rotated.set("m11", f(0.0));
    rotated.set("m10", f(1.0));
    rotated.set("m01", f(-1.0));
    let frame = node(
        1,
        PAGE,
        "FRAME",
        "box",
        &[("frameMaskDisabled", Val::Bool(true))],
    );
    let child = node(2, g(1), "FRAME", "child", &[("transform", rotated)]);
    let wrap = node(
        3,
        PAGE,
        "FRAME",
        "wrap",
        &[
            ("stackMode", e("VERTICAL")),
            ("stackWrap", e("WRAP")),
            ("stackSpacing", f(2.0)),
            ("stackCounterSpacing", f(6.0)),
        ],
    );
    let unset = node(
        5,
        PAGE,
        "FRAME",
        "unset",
        &[
            ("stackMode", e("HORIZONTAL")),
            ("stackWrap", e("WRAP")),
            ("stackSpacing", f(3.0)),
            ("stackCounterSpacing", f(f64::NAN)),
        ],
    );
    let hidden = node(6, g(1), "FRAME", "hidden", &[("visible", Val::Bool(false))]);
    let doc = doc_with(vec![frame, child, wrap, unset, hidden]);
    assert!(!code(&doc, "9:1").contains("hidden"));
    let unset = code(&doc, "9:5");
    assert!(
        unset.contains("flex flex-wrap content-start gap-[3px] items-start"),
        "NaN counter spacing is unset: {unset}"
    );
    let out = code(&doc, "9:1");
    assert!(
        out.starts_with(
            r#"<div data-node-id="9:1" data-name="box" className="w-[10px] h-[10px] relative">"#
        ),
        "{out}"
    );
    assert!(out.contains(r#"className="absolute left-[4px] top-[6px] origin-top-left rotate-[90deg] w-[10px] h-[10px]""#), "{out}");
    let wrap = code(&doc, "9:3");
    assert!(
        wrap.contains("flex flex-col flex-wrap content-start gap-y-[2px] gap-x-[6px] items-start"),
        "{wrap}"
    );
}

#[test]
fn fills_gradients_and_bound_variables() {
    let red = rgba(1.0, 0.0, 0.0, 1.0);
    let half = {
        let mut p = solid(rgba(0.0, 0.0, 1.0, 1.0));
        p.set("opacity", f(0.5));
        p
    };
    let hidden = {
        let mut p = solid(red.clone());
        p.set("visible", Val::Bool(false));
        p
    };
    let a = node(
        1,
        PAGE,
        "RECTANGLE",
        "solid",
        &[("fillPaints", Val::Arr(vec![half, hidden]))],
    );
    let stop = |pos, c| obj(&[("position", f(pos)), ("color", c)]);
    let gradient = obj(&[
        ("type", e("GRADIENT_LINEAR")),
        (
            "stops",
            Val::Arr(vec![
                stop(0.0, red.clone()),
                stop(1.0, rgba(0.0, 1.0, 0.0, 1.0)),
            ]),
        ),
    ]);
    let b = node(
        2,
        PAGE,
        "RECTANGLE",
        "gradient",
        &[("fillPaints", Val::Arr(vec![solid(red.clone()), gradient]))],
    );
    let alias = obj(&[("value", obj(&[("alias", obj(&[("guid", g(20).to_val())]))]))]);
    let bound = {
        let mut p = solid(red);
        p.set("colorVar", alias);
        p
    };
    let c = node(
        3,
        PAGE,
        "RECTANGLE",
        "bound",
        &[("fillPaints", Val::Arr(vec![bound]))],
    );
    let mut variable = node(
        20,
        PAGE,
        "VARIABLE",
        "Brand/Primary 500",
        &[(
            "variableDataValues",
            obj(&[(
                "entries",
                Val::Arr(vec![obj(&[(
                    "variableData",
                    obj(&[("value", obj(&[("colorValue", rgba(0.0, 0.0, 0.0, 1.0))]))]),
                )])]),
            )]),
        )],
    );
    variable.parent = None;
    let doc = doc_with(vec![a, b, c, variable]);
    assert!(code(&doc, "9:1").contains("bg-[#0000FF80]"));
    let out = code(&doc, "9:2");
    assert!(out.contains("[background:linear-gradient(90deg,#FF0000_0%,#00FF00_100%),linear-gradient(#FF0000,#FF0000)]"), "{out}");
    let out = code(&doc, "9:3");
    assert!(
        out.contains("bg-[var(--brand-primary-500,#000000)]"),
        "{out}"
    );
}

#[test]
fn strokes_radii_shadows_blurs_opacity_and_blend() {
    let shadow = obj(&[
        ("type", e("DROP_SHADOW")),
        ("color", rgba(0.0, 0.0, 0.0, 0.25)),
        ("offset", obj(&[("x", f(0.0)), ("y", f(4.0))])),
        ("radius", f(8.0)),
        ("spread", f(1.0)),
    ]);
    let inner = obj(&[
        ("type", e("INNER_SHADOW")),
        ("color", rgba(1.0, 1.0, 1.0, 1.0)),
        ("radius", f(2.0)),
    ]);
    let blur = obj(&[("type", e("BACKGROUND_BLUR")), ("radius", f(10.0))]);
    let rect = node(
        1,
        PAGE,
        "RECTANGLE",
        "card",
        &[
            (
                "strokePaints",
                Val::Arr(vec![solid(rgba(0.0, 0.0, 0.0, 1.0))]),
            ),
            ("strokeWeight", f(2.0)),
            ("strokeAlign", e("INSIDE")),
            ("cornerRadius", f(6.0)),
            ("effects", Val::Arr(vec![shadow, inner, blur])),
            ("opacity", f(0.5)),
            ("blendMode", e("MULTIPLY")),
        ],
    );
    let corners = node(
        2,
        PAGE,
        "RECTANGLE",
        "corners",
        &[
            ("rectangleCornerRadiiIndependent", Val::Bool(true)),
            ("rectangleTopLeftCornerRadius", f(4.0)),
            ("rectangleBottomRightCornerRadius", f(8.0)),
            (
                "strokePaints",
                Val::Arr(vec![solid(rgba(1.0, 0.0, 0.0, 1.0))]),
            ),
            ("strokeWeight", f(1.0)),
            ("strokeAlign", e("CENTER")),
        ],
    );
    let doc = doc_with(vec![rect, corners]);
    let out = code(&doc, "9:1");
    let want = "w-[10px] h-[10px] border-[2px] border-solid border-[color:#000000] rounded-[6px] backdrop-blur-[5px] shadow-[0px_4px_8px_1px_#00000040,inset_0px_0px_2px_0px_#FFFFFF] opacity-[0.5] mix-blend-multiply";
    assert!(out.contains(want), "{out}");
    assert!(code(&doc, "9:2").contains("rounded-tl-[4px] rounded-br-[8px]"));
    assert!(notes(&doc, "9:2").contains("approximations: 1 stroke alignments approximated"));
    assert!(!notes(&doc, "9:1").contains("approximations"));
}

#[test]
fn wrapped_stacks_align_their_lines_with_align_content() {
    use figstudio_fig::Document;
    use figstudio_mcp::tools::design::{Assets, Options, design_context};
    use figstudio_mcp::tools::rendering::Rendering;
    // The public fixture: node 1:2 is a 280x195 horizontal wrap frame of eight 40x40 children.
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/stacks_wrap.fig");
    let doc = Document::open(fixture).unwrap();
    let options = Options {
        file_key: "stacks_wrap".into(),
        assets: Assets::Inline,
        rendering: Rendering::default(),
        ..Default::default()
    };
    let out = texts(&design_context(&doc, "1:2", &options)).remove(0);
    let frame = out
        .lines()
        .find(|l| l.contains("data-node-id=\"1:2\""))
        .unwrap();
    assert!(frame.contains("flex-wrap content-start"), "{frame}");
    // A built wrap frame whose lines spread: stackCounterAlignContent SPACE_BETWEEN.
    let spread = node(
        7,
        PAGE,
        "FRAME",
        "spread",
        &[
            ("stackMode", e("HORIZONTAL")),
            ("stackWrap", e("WRAP")),
            ("stackCounterAlignContent", e("SPACE_BETWEEN")),
            ("stackSpacing", f(3.0)),
            ("stackCounterSpacing", f(6.0)),
        ],
    );
    let doc = doc_with(vec![spread]);
    let code = code(&doc, "9:7");
    assert!(code.contains("flex flex-wrap content-between"), "{code}");
    assert!(!code.contains("content-start"), "{code}");
}

//! Paint lists, stroke style and effects made by the editor draw as specified.

use std::path::PathBuf;

use figstudio_fig::edit::{
    EditOp, EffectSpec, GradientKind, History, PaintSlot, PaintSpec, StopSpec, execute_edit_ops,
};
use figstudio_fig::{Document, Guid};
use figstudio_render::kurbo::Affine;
use figstudio_render::{Renderer, Viewport, node_bounds};

fn open(name: &str) -> Document {
    Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(format!("{name}.fig")),
    )
    .unwrap()
}

fn run(doc: &mut Document, h: &mut History, ops: Vec<EditOp>) -> Vec<Guid> {
    execute_edit_ops(doc, h, &ops, None, None, false)
        .unwrap()
        .0
        .created_ids
}

/// A new page holding one 100 × 50 rectangle at (50, 50).
fn page_with_rect() -> (Document, History, Guid, Guid) {
    let mut doc = open("structure");
    let mut h = History::new(20);
    let page = run(
        &mut doc,
        &mut h,
        vec![EditOp::CreatePage {
            id: None,
            name: "P".into(),
        }],
    )[0];
    let rect = run(
        &mut doc,
        &mut h,
        vec![EditOp::CreateShape {
            id: None,
            kind: "RECTANGLE".into(),
            parent: page,
            name: None,
            x: 50.0,
            y: 50.0,
            width: 100.0,
            height: 50.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }],
    )[0];
    (doc, h, page, rect)
}

/// RGBA of page pixel `(x, y)` in a 200 × 200 render of `page`.
fn sampler(doc: &Document, page: Guid) -> impl Fn(usize, usize) -> [u8; 4] + use<> {
    let frame = Renderer::new().render(
        doc,
        page,
        &Viewport {
            width: 200,
            height: 200,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    let px = frame.pixmap.data_as_u8_slice().to_vec();
    move |x, y| {
        let i = (y * 200 + x) * 4;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    }
}

fn gradient(from: [f64; 2], to: [f64; 2]) -> PaintSpec {
    PaintSpec::Gradient {
        kind: GradientKind::Linear,
        stops: vec![
            StopSpec {
                position: 0.0,
                hex: "#FF0000".into(),
            },
            StopSpec {
                position: 1.0,
                hex: "#0000FF".into(),
            },
        ],
        from: Some(from),
        to: Some(to),
        opacity: None,
        visible: None,
        blend_mode: None,
    }
}

#[test]
fn gradient_handles_set_the_direction() {
    let (mut doc, mut h, page, rect) = page_with_rect();
    let set = |doc: &mut Document, h: &mut History, p| {
        run(
            doc,
            h,
            vec![EditOp::SetPaints {
                target: rect,
                slot: PaintSlot::Fill,
                paints: vec![p],
            }],
        );
    };
    set(&mut doc, &mut h, gradient([0.0, 0.5], [1.0, 0.5]));
    let px = sampler(&doc, page);
    let (l, r) = (px(52, 75), px(147, 75));
    assert!(
        l[0] > 200 && l[2] < 60 && r[2] > 200 && r[0] < 60,
        "{l:?} {r:?}"
    );
    set(&mut doc, &mut h, gradient([0.5, 1.0], [0.5, 0.0]));
    let px = sampler(&doc, page);
    let (top, bottom) = (px(100, 52), px(100, 97));
    assert!(bottom[0] > 200 && top[2] > 200, "{top:?} {bottom:?}");
}

#[test]
fn dashes_leave_gaps_and_a_drop_shadow_draws_outside() {
    let (mut doc, mut h, page, rect) = page_with_rect();
    run(
        &mut doc,
        &mut h,
        vec![
            EditOp::SetPaints {
                target: rect,
                slot: PaintSlot::Fill,
                paints: vec![],
            },
            EditOp::SetPaints {
                target: rect,
                slot: PaintSlot::Stroke,
                paints: vec![PaintSpec::Solid {
                    hex: "#000000".into(),
                    opacity: None,
                    visible: None,
                    blend_mode: None,
                }],
            },
            EditOp::SetStrokeStyle {
                target: rect,
                weight: Some(4.0),
                align: Some("CENTER".into()),
                cap: None,
                join: None,
                miter_limit: None,
                dashes: Some(vec![10.0, 10.0]),
                sides: None,
            },
        ],
    );
    let px = sampler(&doc, page);
    let drawn = (52..148).filter(|&x| px(x, 50)[3] > 128).count();
    assert!(
        drawn > 20 && drawn < 76,
        "{drawn} of 96 top-edge pixels drawn"
    );

    run(
        &mut doc,
        &mut h,
        vec![
            EditOp::SetPaints {
                target: rect,
                slot: PaintSlot::Fill,
                paints: vec![PaintSpec::Solid {
                    hex: "#FFFFFF".into(),
                    opacity: None,
                    visible: None,
                    blend_mode: None,
                }],
            },
            EditOp::AddEffect {
                target: rect,
                effect: EffectSpec::DropShadow {
                    hex: Some("#000000FF".into()),
                    offset: Some([0.0, 20.0]),
                    radius: Some(0.0),
                    spread: None,
                    show_behind: None,
                    visible: None,
                    blend_mode: None,
                },
                index: None,
            },
        ],
    );
    let px = sampler(&doc, page);
    assert!(px(100, 115)[3] > 200, "no shadow below the layer");
}

#[test]
fn a_vector_keeps_its_stroke_after_its_style_changes() {
    let mut doc = open("vector");
    let v: Guid = "0:5".parse().unwrap();
    let count = |doc: &Document| {
        let b = node_bounds(doc, v).unwrap();
        let frame = Renderer::new().render(
            doc,
            v,
            &Viewport {
                width: 400,
                height: 400,
                transform: Affine::scale(400.0 / b.width().max(b.height()))
                    * Affine::translate((-b.x0, -b.y0)),
                background: None,
            },
        );
        frame
            .pixmap
            .data_as_u8_slice()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 100)
            .count()
    };
    let mut h = History::new(5);
    // Only the stroke draws, so the count measures it.
    run(
        &mut doc,
        &mut h,
        vec![EditOp::SetPaints {
            target: v,
            slot: PaintSlot::Fill,
            paints: vec![],
        }],
    );
    let before = count(&doc);
    assert!(before > 1000, "the stroke draws only {before} pixels");
    run(
        &mut doc,
        &mut h,
        vec![EditOp::SetStrokeStyle {
            target: v,
            weight: None,
            align: None,
            cap: Some("ROUND".into()),
            join: None,
            miter_limit: None,
            dashes: None,
            sides: None,
        }],
    );
    let after = count(&doc);
    assert!(
        after * 10 > before * 7,
        "stroke lost: {before} -> {after} pixels"
    );
}

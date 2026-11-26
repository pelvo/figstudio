//! Render and hit-test regressions for PR #90 boolean bounds/transforms and strokeGeometry invalidation.

use std::path::PathBuf;

use figstudio_fig::edit::geometry::{encode_path_commands, make_path_entry, rounded_rect_commands};
use figstudio_fig::edit::styles_vars::hex_paint;
use figstudio_fig::edit::vector_net::{
    BooleanOp, VectorNetwork, VectorRegion, VectorSegment, VectorVertex,
};
use figstudio_fig::edit::{EditOp, History, Txn, execute_edit_ops};
use figstudio_fig::{Document, Guid, Obj, Val, intern};
use figstudio_render::kurbo::{Affine, Point};
use figstudio_render::{Renderer, Viewport, hit_test, node_bounds};

fn empty_page_doc() -> (Document, Guid, History) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let mut doc = Document::open(path).unwrap();
    let mut history = History::new(30);
    let page = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreatePage {
            id: None,
            name: "Geom".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    (doc, page, history)
}

fn pixel(px: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
    let i = (y * w + x) * 4;
    [px[i], px[i + 1], px[i + 2], px[i + 3]]
}

#[test]
fn boolean_combine_off_origin_rotated_operands_renders_and_hits_at_actual_bounds() {
    let (mut doc, page, mut history) = empty_page_doc();

    // Operand A: 80x20 red rect at (400, 80), rotated 90 deg -> occupies [380..400, 80..160].
    let a = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "ROUNDED_RECTANGLE".into(),
            parent: page,
            name: Some("Arm A".into()),
            x: 400.0,
            y: 80.0,
            width: 80.0,
            height: 20.0,
            corner_radius: 0.0,
            fill_hex: Some("#FF0000".into()),
            count: None,
            star_inner_scale: None,
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::Rotate {
            target: a,
            degrees: 90.0,
        }],
        None,
        None,
        false,
    )
    .unwrap();

    // Operand B inside a transparent translated container at (300, 100), placed at local (70, 20),
    // size 60x20 -> occupies [370..430, 120..140] in page space, crossing A.
    let frame = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: Some("Container".into()),
            x: 300.0,
            y: 100.0,
            width: 150.0,
            height: 80.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetSolidFill {
            target: frame,
            hex: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let b = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "ROUNDED_RECTANGLE".into(),
            parent: frame,
            name: Some("Arm B".into()),
            x: 70.0,
            y: 20.0,
            width: 60.0,
            height: 20.0,
            corner_radius: 0.0,
            fill_hex: Some("#0000FF".into()),
            count: None,
            star_inner_scale: None,
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];

    let bool_id = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Union,
            a,
            b,
            name: Some("Cross".into()),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];

    // Bounds must cover the actual combined cross [370..430, 80..160], not (0, 0).
    let bounds = node_bounds(&doc, bool_id).expect("boolean bounds");
    assert!(
        (bounds.x0 - 370.0).abs() < 1.0
            && (bounds.y0 - 80.0).abs() < 1.0
            && (bounds.x1 - 430.0).abs() < 1.0
            && (bounds.y1 - 160.0).abs() < 1.0,
        "unexpected boolean bounds: {bounds:?}"
    );

    // Hit-test inside the rotated vertical arm and horizontal arm must hit (never culled).
    assert!(
        hit_test(&doc, page, Point::new(390.0, 95.0)).is_some(),
        "hit_test missed vertical rotated arm"
    );
    assert!(
        hit_test(&doc, page, Point::new(420.0, 130.0)).is_some(),
        "hit_test missed cross-parent horizontal arm"
    );

    // Render a viewport tightly around [360..440, 70..170] so (0, 0) is strictly off-screen.
    let rendered = Renderer::new().render(
        &doc,
        page,
        &Viewport {
            width: 80,
            height: 100,
            transform: Affine::translate((-360.0, -70.0)),
            background: None,
        },
    );
    let px = rendered.pixmap.data_as_u8_slice();
    // Vertical rotated arm at page (390, 95) -> viewport (30, 25) must be red (A's fill).
    assert_eq!(
        pixel(px, 80, 30, 25),
        [255, 0, 0, 255],
        "rotated vertical arm was not drawn (culled or unrotated)"
    );
    // Horizontal cross-parent arm at page (420, 130) -> viewport (60, 60) must also be red.
    assert_eq!(
        pixel(px, 80, 60, 60),
        [255, 0, 0, 255],
        "cross-parent horizontal arm was not drawn"
    );
    // Unrotated A's former extent at page (450, 90) would be outside, and page (420, 90) -> (60, 20)
    // must stay transparent (not red).
    assert_eq!(
        pixel(px, 80, 60, 20)[3],
        0,
        "unrotated operand footprint leaked into output"
    );
}

#[test]
fn resized_and_restroked_shape_with_stored_stroke_geometry_renders_new_stroke() {
    let (mut doc, page, mut history) = empty_page_doc();

    let style_id = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateFillStyle {
            name: "Red Stroke Style".into(),
            hex: "#FF0000".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];

    // Create a 40x40 un-filled rectangle at (20, 20) with a pre-baked 40x40 strokeGeometry band,
    // independent border weights, and a bound red stroke style.
    let rect_id = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "ROUNDED_RECTANGLE".into(),
            parent: page,
            name: Some("Stroked".into()),
            x: 20.0,
            y: 20.0,
            width: 40.0,
            height: 40.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];

    {
        let mut t = Txn::new("seed pre-baked strokeGeometry");
        // Clear fill so only the stroke draws.
        t.set_field(rect_id, "fillPaints", Val::Arr(Vec::new()));
        t.set_field(
            rect_id,
            "strokePaints",
            Val::Arr(vec![hex_paint("#FF0000").unwrap()]),
        );
        t.set_field(rect_id, "strokeWeight", Val::Float(2.0));
        t.set_field(
            rect_id,
            "strokeAlign",
            Val::Enum(figstudio_fig::intern("INSIDE")),
        );
        t.set_field(rect_id, "borderStrokeWeightsIndependent", Val::Bool(true));
        t.set_field(rect_id, "borderTopWeight", Val::Float(2.0));
        t.set_field(
            rect_id,
            "styleIdForStrokeFill",
            Val::Obj(Obj::from([("guid", style_id.to_val())])),
        );
        // Stored 4-unit centred band around [0..40, 0..40] (even-odd).
        let mut band = rounded_rect_commands(44.0, 44.0, [0.0; 4], 0.0);
        for c in &mut band {
            if let figstudio_fig::edit::geometry::PathCmd::MoveTo(x, y)
            | figstudio_fig::edit::geometry::PathCmd::LineTo(x, y) = c
            {
                *x -= 2.0;
                *y -= 2.0;
            }
        }
        let mut hole = rounded_rect_commands(36.0, 36.0, [0.0; 4], 0.0);
        for c in &mut hole {
            if let figstudio_fig::edit::geometry::PathCmd::MoveTo(x, y)
            | figstudio_fig::edit::geometry::PathCmd::LineTo(x, y) = c
            {
                *x += 2.0;
                *y += 2.0;
            }
        }
        band.extend(hole);
        let blob_idx = doc.blobs.len();
        t.add_blobs.push(encode_path_commands(&band));
        t.set_field(
            rect_id,
            "strokeGeometry",
            Val::Arr(vec![make_path_entry(blob_idx, true)]),
        );
        doc.apply_txn(&t).unwrap();
    }

    let render_frame = |doc: &Document| {
        Renderer::new().render(
            doc,
            page,
            &Viewport {
                width: 140,
                height: 100,
                transform: Affine::IDENTITY,
                background: None,
            },
        )
    };

    // 1. Resize to 80x40 -> stroke must move to new right edge x=99, not stay at old x=59.
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::Resize {
            target: rect_id,
            width: 80.0,
            height: 40.0,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let f1 = render_frame(&doc);
    let px1 = f1.pixmap.data_as_u8_slice();
    assert_eq!(
        pixel(px1, 140, 59, 40)[3],
        0,
        "stale strokeGeometry still drew at old right edge x=59"
    );
    assert_eq!(
        pixel(px1, 140, 40, 20),
        [255, 0, 0, 255],
        "top stroke still visible after resize"
    );

    // 2. SetStroke to green 6px OUTSIDE -> must draw green outside left edge (x=16, y=40),
    // overriding both independent border weights and the old red styleIdForStrokeFill!
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetStroke {
            target: rect_id,
            hex: Some("#00FF00".into()),
            weight: Some(6.0),
            align: Some("OUTSIDE".into()),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let f2 = render_frame(&doc);
    let px2 = f2.pixmap.data_as_u8_slice();
    assert_eq!(
        pixel(px2, 140, 16, 40),
        [0, 255, 0, 255],
        "SetStroke outside green stroke missing at x=16"
    );
    assert_eq!(
        pixel(px2, 140, 102, 40),
        [0, 255, 0, 255],
        "SetStroke outside green right stroke missing at x=102"
    );
    assert_eq!(
        pixel(px2, 140, 22, 40)[3],
        0,
        "inside of box must remain clear with OUTSIDE stroke"
    );

    // 3. Re-seed a sharp stored strokeGeometry and round corners -> corner at (20, 20) clears.
    {
        let mut t = Txn::new("seed sharp 80x40 strokeGeometry");
        let mut band = rounded_rect_commands(84.0, 44.0, [0.0; 4], 0.0);
        for c in &mut band {
            if let figstudio_fig::edit::geometry::PathCmd::MoveTo(x, y)
            | figstudio_fig::edit::geometry::PathCmd::LineTo(x, y) = c
            {
                *x -= 2.0;
                *y -= 2.0;
            }
        }
        let mut hole = rounded_rect_commands(76.0, 36.0, [0.0; 4], 0.0);
        for c in &mut hole {
            if let figstudio_fig::edit::geometry::PathCmd::MoveTo(x, y)
            | figstudio_fig::edit::geometry::PathCmd::LineTo(x, y) = c
            {
                *x += 2.0;
                *y += 2.0;
            }
        }
        band.extend(hole);
        let blob_idx = doc.blobs.len();
        t.add_blobs.push(encode_path_commands(&band));
        t.set_field(
            rect_id,
            "strokeAlign",
            Val::Enum(figstudio_fig::intern("INSIDE")),
        );
        t.set_field(rect_id, "strokeWeight", Val::Float(2.0));
        t.set_field(
            rect_id,
            "strokeGeometry",
            Val::Arr(vec![make_path_entry(blob_idx, true)]),
        );
        doc.apply_txn(&t).unwrap();
    }
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetCornerRadii {
            target: rect_id,
            radii: [20.0; 4],
            smoothing: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let f3 = render_frame(&doc);
    let px3 = f3.pixmap.data_as_u8_slice();
    assert_eq!(
        pixel(px3, 140, 20, 20)[3],
        0,
        "SetCornerRadii left stale sharp corner strokeGeometry at (20, 20)"
    );
    assert_eq!(
        pixel(px3, 140, 60, 20),
        [0, 255, 0, 255],
        "top straight edge still stroked after SetCornerRadii"
    );
}

fn seed_solid_rect_stroke_geom(doc: &mut Document, target: Guid, w: f32, h: f32, half_t: f32) {
    let mut band = rounded_rect_commands(w + half_t * 2.0, h + half_t * 2.0, [0.0; 4], 0.0);
    for c in &mut band {
        if let figstudio_fig::edit::geometry::PathCmd::MoveTo(x, y)
        | figstudio_fig::edit::geometry::PathCmd::LineTo(x, y) = c
        {
            *x -= half_t;
            *y -= half_t;
        }
    }
    let mut hole = rounded_rect_commands(
        (w - half_t * 2.0).max(1.0),
        (h - half_t * 2.0).max(1.0),
        [0.0; 4],
        0.0,
    );
    for c in &mut hole {
        if let figstudio_fig::edit::geometry::PathCmd::MoveTo(x, y)
        | figstudio_fig::edit::geometry::PathCmd::LineTo(x, y) = c
        {
            *x += half_t;
            *y += half_t;
        }
    }
    band.extend(hole);
    let mut t = Txn::new("seed strokeGeometry");
    let blob_idx = doc.blobs.len();
    t.add_blobs.push(encode_path_commands(&band));
    t.set_field(
        target,
        "strokeGeometry",
        Val::Arr(vec![make_path_entry(blob_idx, true)]),
    );
    doc.apply_txn(&t).unwrap();
}

#[test]
fn derived_stroke_invalidation_renders_updated_text_patch_raw_hidden_stroke_and_bind_style() {
    let (mut doc, page, mut history) = empty_page_doc();

    // 1. Stroked TEXT node with stale strokeGeometry: after SetText to "", stale stroke band must not render.
    let text_id = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateText {
            id: None,
            parent: page,
            x: 10.0,
            y: 10.0,
            characters: "Old".into(),
            font_family: "Inter".into(),
            font_style: "Regular".into(),
            font_size: 20.0,
            fill_hex: "#000000".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    {
        let mut t = Txn::new("stroke text with stale strokeGeometry");
        t.set_field(
            text_id,
            "strokePaints",
            Val::Arr(vec![hex_paint("#FF0000").unwrap()]),
        );
        let blob_idx = doc.blobs.len();
        t.add_blobs
            .push(encode_path_commands(&rounded_rect_commands(
                40.0, 20.0, [0.0; 4], 0.0,
            )));
        t.set_field(
            text_id,
            "strokeGeometry",
            Val::Arr(vec![make_path_entry(blob_idx, false)]),
        );
        doc.apply_txn(&t).unwrap();
    }
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetText {
            target: text_id,
            characters: String::new(),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let r_text = Renderer::new().render(
        &doc,
        page,
        &Viewport {
            width: 80,
            height: 50,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    assert_eq!(
        pixel(r_text.pixmap.data_as_u8_slice(), 80, 25, 20)[3],
        0,
        "stale TEXT strokeGeometry still rendered after SetText"
    );

    // 2. PatchRaw unsetting `borderStrokeWeightsIndependent` (with `strokeWeight: 6.0` OUTSIDE)
    // must invalidate stale narrow strokeGeometry so the fresh 6px OUTSIDE stroke renders.
    let rect_id = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "ROUNDED_RECTANGLE".into(),
            parent: page,
            name: None,
            x: 20.0,
            y: 20.0,
            width: 80.0,
            height: 40.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    {
        let mut t = Txn::new("setup stroked rect with independent borders and dashPattern");
        t.set_field(rect_id, "fillPaints", Val::Arr(Vec::new()));
        t.set_field(
            rect_id,
            "strokePaints",
            Val::Arr(vec![hex_paint("#FF0000").unwrap()]),
        );
        t.set_field(rect_id, "strokeWeight", Val::Float(2.0));
        t.set_field(rect_id, "strokeAlign", Val::Enum(intern("CENTER")));
        t.set_field(
            rect_id,
            "dashPattern",
            Val::Arr(vec![Val::Float(6.0), Val::Float(6.0)]),
        );
        t.set_field(rect_id, "borderStrokeWeightsIndependent", Val::Bool(true));
        t.set_field(rect_id, "borderTopWeight", Val::Float(1.0));
        doc.apply_txn(&t).unwrap();
    }
    seed_solid_rect_stroke_geom(&mut doc, rect_id, 80.0, 40.0, 1.0);
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::PatchRaw {
            target: rect_id,
            set: Default::default(),
            unset: vec![
                "borderStrokeWeightsIndependent".into(),
                "borderTopWeight".into(),
            ],
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let r_dash = Renderer::new().render(
        &doc,
        page,
        &Viewport {
            width: 120,
            height: 80,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    let px_dash = r_dash.pixmap.data_as_u8_slice();
    let lit = (26..94)
        .filter(|&x| pixel(px_dash, 120, x, 20) == [255, 0, 0, 255])
        .count();
    assert!(
        lit > 15 && lit < 55,
        "PatchRaw(unset independent borders) must invalidate stale solid strokeGeometry so dashed top edge renders (lit={lit}/68)"
    );

    // 3. Hidden stroke paint (`visible: false`) + stale narrow strokeGeometry, then SetStroke
    // color-only (keeping strokeWeight=6, OUTSIDE): fresh 6px green band must draw outside edge.
    {
        let mut hidden = hex_paint("#FF0000").unwrap().into_owned();
        hidden.set("visible", Val::Bool(false));
        let mut t = Txn::new("hidden stroke with weight 6 OUTSIDE");
        t.set_field(rect_id, "strokePaints", Val::Arr(vec![hidden]));
        t.set_field(rect_id, "strokeWeight", Val::Float(6.0));
        t.set_field(rect_id, "strokeAlign", Val::Enum(intern("OUTSIDE")));
        t.unset_field(rect_id, "dashPattern");
        doc.apply_txn(&t).unwrap();
    }
    seed_solid_rect_stroke_geom(&mut doc, rect_id, 80.0, 40.0, 1.0);
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetStroke {
            target: rect_id,
            hex: Some("#00FF00".into()),
            weight: None,
            align: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let r_hidden = Renderer::new().render(
        &doc,
        page,
        &Viewport {
            width: 120,
            height: 80,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    assert_eq!(
        pixel(r_hidden.pixmap.data_as_u8_slice(), 120, 16, 40),
        [0, 255, 0, 255],
        "SetStroke over hidden paint must render fresh 6px OUTSIDE green stroke at x=16"
    );

    // 4. BindStyle(stroke) on unstroked node with stale narrow strokeGeometry renders fresh 6px blue stroke.
    let blue_style = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateFillStyle {
            name: "Blue Stroke".into(),
            hex: "#0000FF".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    {
        let mut t = Txn::new("clear strokePaints");
        t.set_field(rect_id, "strokePaints", Val::Arr(Vec::new()));
        doc.apply_txn(&t).unwrap();
    }
    seed_solid_rect_stroke_geom(&mut doc, rect_id, 80.0, 40.0, 1.0);
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::BindStyle {
            target: rect_id,
            slot: "stroke".into(),
            style: Some(blue_style),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let r_bind = Renderer::new().render(
        &doc,
        page,
        &Viewport {
            width: 120,
            height: 80,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    assert_eq!(
        pixel(r_bind.pixmap.data_as_u8_slice(), 120, 16, 40),
        [0, 0, 255, 255],
        "BindStyle(stroke) must render fresh 6px OUTSIDE blue stroke at x=16"
    );
}

#[test]
fn boolean_combine_holes_curves_rotated_parent_and_collinear_seams_render_accurately() {
    let (mut doc, page, mut history) = empty_page_doc();

    // 1. Hollow ring Subtract(100x100, 40x40) combined with a third shape via Union and Subtract:
    // center of hole at (50, 50) must stay transparent!
    let (rep, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::CreateShape {
                id: None,
                kind: "ROUNDED_RECTANGLE".into(),
                parent: page,
                name: None,
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
                corner_radius: 0.0,
                fill_hex: Some("#FF0000".into()),
                count: None,
                star_inner_scale: None,
            },
            EditOp::CreateShape {
                id: None,
                kind: "ROUNDED_RECTANGLE".into(),
                parent: page,
                name: None,
                x: 30.0,
                y: 30.0,
                width: 40.0,
                height: 40.0,
                corner_radius: 0.0,
                fill_hex: Some("#00FF00".into()),
                count: None,
                star_inner_scale: None,
            },
            EditOp::CreateShape {
                id: None,
                kind: "ROUNDED_RECTANGLE".into(),
                parent: page,
                name: None,
                x: 90.0,
                y: 35.0,
                width: 30.0,
                height: 30.0,
                corner_radius: 0.0,
                fill_hex: Some("#0000FF".into()),
                count: None,
                star_inner_scale: None,
            },
            EditOp::CreateShape {
                id: None,
                kind: "ROUNDED_RECTANGLE".into(),
                parent: page,
                name: None,
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 20.0,
                corner_radius: 0.0,
                fill_hex: Some("#FFFF00".into()),
                count: None,
                star_inner_scale: None,
            },
        ],
        None,
        None,
        false,
    )
    .unwrap();
    let (outer, inner, tab, notch) = (
        rep.created_ids[0],
        rep.created_ids[1],
        rep.created_ids[2],
        rep.created_ids[3],
    );
    let hollow = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Subtract,
            a: outer,
            b: inner,
            name: None,
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let with_tab = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Union,
            a: hollow,
            b: tab,
            name: None,
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Subtract,
            a: with_tab,
            b: notch,
            name: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let r1 = Renderer::new().render(
        &doc,
        page,
        &Viewport {
            width: 140,
            height: 120,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    let px1 = r1.pixmap.data_as_u8_slice();
    assert_eq!(
        pixel(px1, 140, 50, 50)[3],
        0,
        "hole center (50, 50) filled solid after chained booleans"
    );
    assert_eq!(
        pixel(px1, 140, 10, 10)[3],
        0,
        "subtracted corner notch must be clear"
    );
    assert_eq!(
        pixel(px1, 140, 15, 50),
        [255, 0, 0, 255],
        "left wall of ring"
    );
    assert_eq!(
        pixel(px1, 140, 110, 50),
        [255, 0, 0, 255],
        "unioned right tab"
    );

    // 2. Donut ELLIPSE (innerRadius=0.5) unioned with a side rect keeps center transparent.
    let (mut doc_d, page_d, mut hist_d) = empty_page_doc();
    let (drep, _) = execute_edit_ops(
        &mut doc_d,
        &mut hist_d,
        &[
            EditOp::CreateShape {
                id: None,
                kind: "ELLIPSE".into(),
                parent: page_d,
                name: None,
                x: 10.0,
                y: 10.0,
                width: 100.0,
                height: 100.0,
                corner_radius: 0.0,
                fill_hex: Some("#0000FF".into()),
                count: None,
                star_inner_scale: None,
            },
            EditOp::CreateShape {
                id: None,
                kind: "ROUNDED_RECTANGLE".into(),
                parent: page_d,
                name: None,
                x: 100.0,
                y: 45.0,
                width: 30.0,
                height: 30.0,
                corner_radius: 0.0,
                fill_hex: Some("#FF0000".into()),
                count: None,
                star_inner_scale: None,
            },
        ],
        None,
        None,
        false,
    )
    .unwrap();
    let (donut, dtab) = (drep.created_ids[0], drep.created_ids[1]);
    {
        let mut t = Txn::new("set donut arcData and clear stale full-ellipse fillGeometry");
        t.set_field(
            donut,
            "arcData",
            Val::Obj(Obj::from([
                ("startingAngle", Val::Float(0.0)),
                ("endingAngle", Val::Float(std::f64::consts::TAU)),
                ("innerRadius", Val::Float(0.5)),
            ])),
        );
        t.unset_field(donut, "fillGeometry");
        doc_d.apply_txn(&t).unwrap();
    }
    execute_edit_ops(
        &mut doc_d,
        &mut hist_d,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Union,
            a: donut,
            b: dtab,
            name: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let rd = Renderer::new().render(
        &doc_d,
        page_d,
        &Viewport {
            width: 140,
            height: 120,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    let pxd = rd.pixmap.data_as_u8_slice();
    assert_eq!(
        pixel(pxd, 140, 60, 60)[3],
        0,
        "donut ELLIPSE hole center filled solid after Union"
    );
    assert_eq!(
        pixel(pxd, 140, 22, 60),
        [0, 0, 255, 255],
        "donut ring filled"
    );
    assert_eq!(
        pixel(pxd, 140, 120, 60),
        [0, 0, 255, 255],
        "donut side tab filled"
    );

    // 3. Curved ELLIPSE + 35-deg rotated STAR inside a 30-deg rotated FRAME, plus VECTOR
    // with normalizedSize and no fillGeometry.
    let (mut doc_c, page_c, mut hist_c) = empty_page_doc();
    let frame = execute_edit_ops(
        &mut doc_c,
        &mut hist_c,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page_c,
            name: None,
            x: 80.0,
            y: 40.0,
            width: 180.0,
            height: 180.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    execute_edit_ops(
        &mut doc_c,
        &mut hist_c,
        &[
            EditOp::SetSolidFill {
                target: frame,
                hex: None,
            },
            EditOp::Rotate {
                target: frame,
                degrees: 30.0,
            },
        ],
        None,
        None,
        false,
    )
    .unwrap();
    let (crep, _) = execute_edit_ops(
        &mut doc_c,
        &mut hist_c,
        &[
            EditOp::CreateShape {
                id: None,
                kind: "ELLIPSE".into(),
                parent: frame,
                name: None,
                x: 20.0,
                y: 20.0,
                width: 80.0,
                height: 80.0,
                corner_radius: 0.0,
                fill_hex: Some("#00AA00".into()),
                count: None,
                star_inner_scale: None,
            },
            EditOp::CreateShape {
                id: None,
                kind: "STAR".into(),
                parent: frame,
                name: None,
                x: 70.0,
                y: 20.0,
                width: 80.0,
                height: 80.0,
                corner_radius: 0.0,
                fill_hex: Some("#FF0000".into()),
                count: Some(5),
                star_inner_scale: Some(0.45),
            },
        ],
        None,
        None,
        false,
    )
    .unwrap();
    let (ell, star) = (crep.created_ids[0], crep.created_ids[1]);
    execute_edit_ops(
        &mut doc_c,
        &mut hist_c,
        &[EditOp::Rotate {
            target: star,
            degrees: 35.0,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let bool_curved = execute_edit_ops(
        &mut doc_c,
        &mut hist_c,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Union,
            a: ell,
            b: star,
            name: Some("CurvedUnion".into()),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let cb = node_bounds(&doc_c, bool_curved).expect("curved boolean bounds");
    assert!(
        cb.width() > 90.0 && cb.height() > 70.0,
        "unexpected bounds {cb:?}"
    );
    let ell_center = Affine::translate((80.0, 40.0))
        * Affine::rotate(30.0_f64.to_radians())
        * Point::new(60.0, 60.0);
    assert!(
        hit_test(&doc_c, page_c, ell_center).is_some(),
        "hit_test missed ellipse center inside rotated frame"
    );
    let rc = Renderer::new().render(
        &doc_c,
        page_c,
        &Viewport {
            width: 300,
            height: 260,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    assert_eq!(
        pixel(
            rc.pixmap.data_as_u8_slice(),
            300,
            ell_center.x.round() as usize,
            ell_center.y.round() as usize,
        ),
        [0, 170, 0, 255],
        "curved boolean inside rotated frame missing fill at ellipse center"
    );

    // 4. All four ops (Union, Subtract, Intersect, Exclude) on two shapes sharing a collinear boundary edge.
    for (op, left_px, overlap_px, right_px, seam_px) in [
        // A=[10..60, 10..60], B=[35..85, 10..60]: shared top y=10 and bottom y=60 edges, overlap=[35..60].
        (BooleanOp::Union, true, true, true, true),
        (BooleanOp::Subtract, true, false, false, false),
        (BooleanOp::Intersect, false, true, false, false),
        (BooleanOp::Exclude, true, false, true, false),
    ] {
        let (mut doc_s, page_s, mut hist_s) = empty_page_doc();
        let (srep, _) = execute_edit_ops(
            &mut doc_s,
            &mut hist_s,
            &[
                EditOp::CreateShape {
                    id: None,
                    kind: "ROUNDED_RECTANGLE".into(),
                    parent: page_s,
                    name: None,
                    x: 10.0,
                    y: 10.0,
                    width: 50.0,
                    height: 50.0,
                    corner_radius: 0.0,
                    fill_hex: Some("#FF0000".into()),
                    count: None,
                    star_inner_scale: None,
                },
                EditOp::CreateShape {
                    id: None,
                    kind: "ROUNDED_RECTANGLE".into(),
                    parent: page_s,
                    name: None,
                    x: 35.0,
                    y: 10.0,
                    width: 50.0,
                    height: 50.0,
                    corner_radius: 0.0,
                    fill_hex: Some("#0000FF".into()),
                    count: None,
                    star_inner_scale: None,
                },
            ],
            None,
            None,
            false,
        )
        .unwrap();
        execute_edit_ops(
            &mut doc_s,
            &mut hist_s,
            &[EditOp::BooleanCombine {
                operation: op,
                a: srep.created_ids[0],
                b: srep.created_ids[1],
                name: None,
            }],
            None,
            None,
            false,
        )
        .unwrap();
        let rs = Renderer::new().render(
            &doc_s,
            page_s,
            &Viewport {
                width: 100,
                height: 80,
                transform: Affine::IDENTITY,
                background: None,
            },
        );
        let pxs = rs.pixmap.data_as_u8_slice();
        let filled = |x, y| pixel(pxs, 100, x, y) == [255, 0, 0, 255];
        assert_eq!(filled(20, 35), left_px, "{op:?}: left-only pixel mismatch");
        assert_eq!(filled(47, 35), overlap_px, "{op:?}: overlap pixel mismatch");
        assert_eq!(
            filled(72, 35),
            right_px,
            "{op:?}: right-only pixel mismatch"
        );
        assert_eq!(
            pixel(pxs, 100, 47, 12)[3] > 0,
            seam_px || overlap_px,
            "{op:?}: pixel inside shared collinear top edge mismatch"
        );
        assert_eq!(
            pixel(pxs, 100, 47, 5)[3],
            0,
            "{op:?}: pixel outside shared collinear top edge leaked"
        );
    }

    // 5. Diagonal Exclude (XOR) with pinch vertices at (70, 40) and (40, 70):
    // A=[10..70, 10..70], B=[40..100, 40..100] -> overlap [40..70, 40..70] must be transparent.
    let (mut doc_x, page_x, mut hist_x) = empty_page_doc();
    let (xrep, _) = execute_edit_ops(
        &mut doc_x,
        &mut hist_x,
        &[
            EditOp::CreateShape {
                id: None,
                kind: "ROUNDED_RECTANGLE".into(),
                parent: page_x,
                name: None,
                x: 10.0,
                y: 10.0,
                width: 60.0,
                height: 60.0,
                corner_radius: 0.0,
                fill_hex: Some("#FF0000".into()),
                count: None,
                star_inner_scale: None,
            },
            EditOp::CreateShape {
                id: None,
                kind: "ROUNDED_RECTANGLE".into(),
                parent: page_x,
                name: None,
                x: 40.0,
                y: 40.0,
                width: 60.0,
                height: 60.0,
                corner_radius: 0.0,
                fill_hex: Some("#0000FF".into()),
                count: None,
                star_inner_scale: None,
            },
        ],
        None,
        None,
        false,
    )
    .unwrap();
    execute_edit_ops(
        &mut doc_x,
        &mut hist_x,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Exclude,
            a: xrep.created_ids[0],
            b: xrep.created_ids[1],
            name: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let rx = Renderer::new().render(
        &doc_x,
        page_x,
        &Viewport {
            width: 120,
            height: 120,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    let pxx = rx.pixmap.data_as_u8_slice();
    assert_eq!(
        pixel(pxx, 120, 25, 25),
        [255, 0, 0, 255],
        "XOR upper-left lobe"
    );
    assert_eq!(
        pixel(pxx, 120, 85, 85),
        [255, 0, 0, 255],
        "XOR lower-right lobe"
    );
    assert_eq!(
        pixel(pxx, 120, 55, 55)[3],
        0,
        "XOR center overlap must be empty"
    );

    // 6. VECTOR with normalizedSize and NO fillGeometry + ODD hole combined with Union:
    // verifies rendered fill pixels, transparent hole center, node_bounds, and hit_test.
    let vpath = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/vector.fig");
    let mut vdoc = Document::open(vpath).unwrap();
    let vec_id = vdoc.nodes.iter().find(|n| n.kind == "VECTOR").unwrap().id;
    let vpage = vdoc.page_of(vec_id).expect("page");
    let mut vhist = History::new(20);
    let odd_net = VectorNetwork {
        vertices: vec![
            VectorVertex {
                style: 0,
                x: 0.0,
                y: 0.0,
            },
            VectorVertex {
                style: 0,
                x: 10.0,
                y: 0.0,
            },
            VectorVertex {
                style: 0,
                x: 10.0,
                y: 10.0,
            },
            VectorVertex {
                style: 0,
                x: 0.0,
                y: 10.0,
            },
            VectorVertex {
                style: 0,
                x: 3.0,
                y: 3.0,
            },
            VectorVertex {
                style: 0,
                x: 7.0,
                y: 3.0,
            },
            VectorVertex {
                style: 0,
                x: 7.0,
                y: 7.0,
            },
            VectorVertex {
                style: 0,
                x: 3.0,
                y: 7.0,
            },
        ],
        segments: vec![
            VectorSegment {
                style: 0,
                start: 0,
                tangent_start: (0.0, 0.0),
                end: 1,
                tangent_end: (0.0, 0.0),
            },
            VectorSegment {
                style: 0,
                start: 1,
                tangent_start: (0.0, 0.0),
                end: 2,
                tangent_end: (0.0, 0.0),
            },
            VectorSegment {
                style: 0,
                start: 2,
                tangent_start: (0.0, 0.0),
                end: 3,
                tangent_end: (0.0, 0.0),
            },
            VectorSegment {
                style: 0,
                start: 3,
                tangent_start: (0.0, 0.0),
                end: 0,
                tangent_end: (0.0, 0.0),
            },
            VectorSegment {
                style: 0,
                start: 4,
                tangent_start: (0.0, 0.0),
                end: 5,
                tangent_end: (0.0, 0.0),
            },
            VectorSegment {
                style: 0,
                start: 5,
                tangent_start: (0.0, 0.0),
                end: 6,
                tangent_end: (0.0, 0.0),
            },
            VectorSegment {
                style: 0,
                start: 6,
                tangent_start: (0.0, 0.0),
                end: 7,
                tangent_end: (0.0, 0.0),
            },
            VectorSegment {
                style: 0,
                start: 7,
                tangent_start: (0.0, 0.0),
                end: 4,
                tangent_end: (0.0, 0.0),
            },
        ],
        regions: vec![VectorRegion {
            winding_nonzero: false,
            style_id: 0,
            loops: vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]],
        }],
    };
    execute_edit_ops(
        &mut vdoc,
        &mut vhist,
        &[EditOp::SetVectorNetwork {
            target: vec_id,
            network: odd_net,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    // Place on a fresh empty page with size 100x100 vs normalizedSize 10x10 and NO fillGeometry.
    let fresh_page = execute_edit_ops(
        &mut vdoc,
        &mut vhist,
        &[EditOp::CreatePage {
            id: None,
            name: "VecBool".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let _ = vpage;
    {
        let mut t = Txn::new("move vector to fresh page with normalizedSize fallback");
        t.set_field(
            vec_id,
            "parentIndex",
            Val::Obj(Obj::from([
                ("guid", fresh_page.to_val()),
                ("position", Val::Str("a".into())),
            ])),
        );
        t.set_field(
            vec_id,
            "transform",
            Val::Obj(Obj::from([
                ("m00", Val::Float(1.0)),
                ("m01", Val::Float(0.0)),
                ("m02", Val::Float(10.0)),
                ("m10", Val::Float(0.0)),
                ("m11", Val::Float(1.0)),
                ("m12", Val::Float(10.0)),
            ])),
        );
        t.set_field(
            vec_id,
            "size",
            Val::Obj(Obj::from([
                ("x", Val::Float(100.0)),
                ("y", Val::Float(100.0)),
            ])),
        );
        t.set_field(
            vec_id,
            "fillPaints",
            Val::Arr(vec![hex_paint("#00AA00").unwrap()]),
        );
        t.set_field(vec_id, "strokePaints", Val::Arr(Vec::new()));
        t.unset_field(vec_id, "fillGeometry");
        vdoc.apply_txn(&t).unwrap();
    }
    let vtab = execute_edit_ops(
        &mut vdoc,
        &mut vhist,
        &[EditOp::CreateShape {
            id: None,
            kind: "ROUNDED_RECTANGLE".into(),
            parent: fresh_page,
            name: None,
            x: 100.0,
            y: 45.0,
            width: 30.0,
            height: 30.0,
            corner_radius: 0.0,
            fill_hex: Some("#FF0000".into()),
            count: None,
            star_inner_scale: None,
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let vbool = execute_edit_ops(
        &mut vdoc,
        &mut vhist,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Union,
            a: vec_id,
            b: vtab,
            name: None,
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let vb = node_bounds(&vdoc, vbool).expect("vector boolean bounds");
    assert!(
        (vb.x0 - 10.0).abs() < 1.0
            && (vb.y0 - 10.0).abs() < 1.0
            && (vb.x1 - 130.0).abs() < 1.0
            && (vb.y1 - 110.0).abs() < 1.0,
        "unexpected vector boolean bounds: {vb:?}"
    );
    assert!(
        hit_test(&vdoc, fresh_page, Point::new(120.0, 60.0)).is_some(),
        "hit_test missed unioned tab"
    );
    let rv = Renderer::new().render(
        &vdoc,
        fresh_page,
        &Viewport {
            width: 150,
            height: 130,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    let pxv = rv.pixmap.data_as_u8_slice();
    assert_eq!(
        pixel(pxv, 150, 60, 60)[3],
        0,
        "ODD-vector hole center must stay transparent"
    );
    assert_eq!(
        pixel(pxv, 150, 20, 60),
        [0, 170, 0, 255],
        "ODD-vector ring wall filled"
    );
    assert_eq!(
        pixel(pxv, 150, 120, 60),
        [0, 170, 0, 255],
        "unioned tab filled"
    );
}

#[test]
fn set_vector_network_updates_cull_bounds_and_renders_region_style_overrides() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/vector.fig");
    let mut doc = Document::open(path).unwrap();
    let vec_id = doc
        .nodes
        .iter()
        .find(|n| n.kind == "VECTOR")
        .expect("VECTOR")
        .id;
    let page = doc.page_of(vec_id).expect("page of vector");
    let mut history = History::new(20);

    // Move vector to page root at (0, 0) with a small 20x20 box, red default fill, and blue styleID=1.
    {
        let mut t = Txn::new("setup 20x20 multi-style vector");
        t.set_field(
            vec_id,
            "parentIndex",
            Val::Obj(Obj::from([
                ("guid", page.to_val()),
                ("position", Val::Str("~".into())),
            ])),
        );
        t.set_field(
            vec_id,
            "transform",
            Val::Obj(Obj::from([
                ("m00", Val::Float(1.0)),
                ("m01", Val::Float(0.0)),
                ("m02", Val::Float(0.0)),
                ("m10", Val::Float(0.0)),
                ("m11", Val::Float(1.0)),
                ("m12", Val::Float(0.0)),
            ])),
        );
        t.set_field(
            vec_id,
            "size",
            Val::Obj(Obj::from([
                ("x", Val::Float(20.0)),
                ("y", Val::Float(20.0)),
            ])),
        );
        t.set_field(
            vec_id,
            "fillPaints",
            Val::Arr(vec![hex_paint("#FF0000").unwrap()]),
        );
        t.set_field(vec_id, "strokePaints", Val::Arr(Vec::new()));
        t.set_field(
            vec_id,
            "vectorData",
            Val::Obj(Obj::from([
                (
                    "normalizedSize",
                    Val::Obj(Obj::from([
                        ("x", Val::Float(20.0)),
                        ("y", Val::Float(20.0)),
                    ])),
                ),
                (
                    "styleOverrideTable",
                    Val::Arr(vec![Val::Obj(Obj::from([
                        ("styleID", Val::Int(1)),
                        ("fillPaints", Val::Arr(vec![hex_paint("#0000FF").unwrap()])),
                    ]))]),
                ),
            ])),
        );
        doc.apply_txn(&t).unwrap();
    }

    // Replace network with two triangles spanning [0..200, 0..100]:
    // upper-right triangle uses style_id 1 (blue), lower-left triangle uses style_id 0 (red).
    let net = VectorNetwork {
        vertices: vec![
            VectorVertex {
                style: 0,
                x: 0.0,
                y: 0.0,
            },
            VectorVertex {
                style: 0,
                x: 200.0,
                y: 0.0,
            },
            VectorVertex {
                style: 0,
                x: 200.0,
                y: 100.0,
            },
            VectorVertex {
                style: 0,
                x: 0.0,
                y: 100.0,
            },
        ],
        segments: vec![
            VectorSegment {
                style: 0,
                start: 0,
                tangent_start: (0.0, 0.0),
                end: 1,
                tangent_end: (0.0, 0.0),
            },
            VectorSegment {
                style: 0,
                start: 1,
                tangent_start: (0.0, 0.0),
                end: 2,
                tangent_end: (0.0, 0.0),
            },
            VectorSegment {
                style: 0,
                start: 2,
                tangent_start: (0.0, 0.0),
                end: 3,
                tangent_end: (0.0, 0.0),
            },
            VectorSegment {
                style: 0,
                start: 3,
                tangent_start: (0.0, 0.0),
                end: 0,
                tangent_end: (0.0, 0.0),
            },
            VectorSegment {
                style: 0,
                start: 0,
                tangent_start: (0.0, 0.0),
                end: 2,
                tangent_end: (0.0, 0.0),
            },
        ],
        regions: vec![
            VectorRegion {
                winding_nonzero: true,
                style_id: 1,
                loops: vec![vec![0, 1, 4]],
            },
            VectorRegion {
                winding_nonzero: true,
                style_id: 0,
                loops: vec![vec![4, 2, 3]],
            },
        ],
    };

    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetVectorNetwork {
            target: vec_id,
            network: net,
        }],
        None,
        None,
        false,
    )
    .unwrap();

    // Render on viewport [60..220, 0..120], strictly excluding the old 20x20 box!
    let rendered = Renderer::new().render(
        &doc,
        page,
        &Viewport {
            width: 160,
            height: 120,
            transform: Affine::translate((-60.0, 0.0)),
            background: None,
        },
    );
    let px = rendered.pixmap.data_as_u8_slice();
    // Page (180, 20) -> viewport (120, 20): inside upper-right triangle -> blue (styleID=1).
    assert_eq!(
        pixel(px, 160, 120, 20),
        [0, 0, 255, 255],
        "upper-right region at (180, 20) was culled or lost styleID=1 blue override"
    );
    // Page (80, 80) -> viewport (20, 80): inside lower-left triangle -> red (styleID=0).
    assert_eq!(
        pixel(px, 160, 20, 80),
        [255, 0, 0, 255],
        "lower-left region at (80, 80) was culled or miscolored"
    );
}

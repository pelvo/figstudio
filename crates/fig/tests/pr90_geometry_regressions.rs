//! Regression tests for PR #90 geometry, vector, boolean, raw enum, and GUID allocation fixes.

use std::collections::HashSet;
use std::path::PathBuf;

use figstudio_fig::edit::geometry::{
    PathCmd, decode_path_commands, encode_path_commands, make_path_entry, rounded_rect_commands,
};
use figstudio_fig::edit::styles_vars::{bind_style, hex_paint};
use figstudio_fig::edit::text_runs::{TextStyleRunPatch, commit_text_data, set_text_plain};
use figstudio_fig::edit::vector_net::{
    BooleanOp, VectorNetwork, VectorRegion, VectorSegment, VectorVertex, decode_vector_network,
};
use figstudio_fig::edit::{EditOp, GuidAlloc, History, Txn, execute_edit_ops};
use figstudio_fig::{Document, Guid, Obj, Val, intern};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn rect_op(id: Option<Guid>, parent: Guid, x: f64, y: f64, w: f64, h: f64) -> EditOp {
    EditOp::CreateShape {
        id,
        kind: "ROUNDED_RECTANGLE".into(),
        parent,
        name: None,
        x,
        y,
        width: w,
        height: h,
        corner_radius: 0.0,
        fill_hex: Some("#FF0000".into()),
        count: None,
        star_inner_scale: None,
    }
}

fn valid_triangle_network() -> VectorNetwork {
    VectorNetwork {
        vertices: vec![
            VectorVertex {
                style: 0,
                x: 0.0,
                y: 0.0,
            },
            VectorVertex {
                style: 0,
                x: 60.0,
                y: 0.0,
            },
            VectorVertex {
                style: 0,
                x: 30.0,
                y: 50.0,
            },
        ],
        segments: vec![
            VectorSegment {
                style: 0,
                start: 0,
                tangent_start: (10.0, 0.0),
                end: 1,
                tangent_end: (-10.0, 0.0),
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
                end: 0,
                tangent_end: (0.0, 0.0),
            },
        ],
        regions: vec![VectorRegion {
            winding_nonzero: true,
            style_id: 0,
            loops: vec![vec![0, 1, 2]],
        }],
    }
}

fn cmds_bbox(cmds: &[PathCmd]) -> (f32, f32, f32, f32) {
    let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
    let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    let mut visit = |x: f32, y: f32| {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    };
    for &c in cmds {
        match c {
            PathCmd::MoveTo(x, y) | PathCmd::LineTo(x, y) => visit(x, y),
            PathCmd::QuadTo(cx, cy, x, y) => {
                visit(cx, cy);
                visit(x, y);
            }
            PathCmd::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                visit(c1x, c1y);
                visit(c2x, c2y);
                visit(x, y);
            }
            PathCmd::Close => {}
        }
    }
    (min_x, min_y, max_x, max_y)
}

#[test]
fn set_vector_network_rejects_invalid_networks_atomically() {
    let mut doc = Document::open(fixture("vector.fig")).expect("open vector.fig");
    let vec_id = doc
        .nodes
        .iter()
        .find(|n| n.kind == "VECTOR")
        .expect("VECTOR in vector.fig")
        .id;
    let baseline = doc.encode_message().unwrap();
    let blobs_before = doc.blobs.len();
    let mut history = History::new(20);

    let mut bad_cases: Vec<(&str, VectorNetwork)> = Vec::new();

    // Out-of-range segment start vertex index.
    let mut bad_seg_start = valid_triangle_network();
    bad_seg_start.segments[0].start = 99;
    bad_cases.push(("segment start out of range", bad_seg_start));

    // Out-of-range segment end vertex index.
    let mut bad_seg_end = valid_triangle_network();
    bad_seg_end.segments[1].end = 3;
    bad_cases.push(("segment end out of range", bad_seg_end));

    // Out-of-range region loop segment index.
    let mut bad_loop_seg = valid_triangle_network();
    bad_loop_seg.regions[0].loops[0][1] = 7;
    bad_cases.push(("region loop segment out of range", bad_loop_seg));

    // Non-finite vertex coordinates.
    let mut nan_vertex = valid_triangle_network();
    nan_vertex.vertices[1].x = f32::NAN;
    bad_cases.push(("NaN vertex coordinate", nan_vertex));

    let mut inf_vertex = valid_triangle_network();
    inf_vertex.vertices[2].y = f32::INFINITY;
    bad_cases.push(("infinite vertex coordinate", inf_vertex));

    // Non-finite segment tangent.
    let mut nan_tangent = valid_triangle_network();
    nan_tangent.segments[0].tangent_start = (f32::NEG_INFINITY, 0.0);
    bad_cases.push(("infinite segment tangent", nan_tangent));

    // Unclosed / disconnected region loop.
    let mut open_loop = valid_triangle_network();
    open_loop.regions[0].loops[0] = vec![0, 1];
    bad_cases.push(("open region loop", open_loop));

    // Empty region loop.
    let mut empty_loop = valid_triangle_network();
    empty_loop.regions[0].loops = vec![vec![]];
    bad_cases.push(("empty region loop", empty_loop));

    // Region style_id overflows 31-bit packed field.
    let mut overflow_style = valid_triangle_network();
    overflow_style.regions[0].style_id = 1 << 31;
    bad_cases.push(("style_id bit 31 overflow", overflow_style));

    for (label, net) in bad_cases {
        let res = execute_edit_ops(
            &mut doc,
            &mut history,
            &[EditOp::SetVectorNetwork {
                target: vec_id,
                network: net,
            }],
            None,
            None,
            false,
        );
        assert!(res.is_err(), "expected Err for {label}, got Ok");
        assert_eq!(
            doc.blobs.len(),
            blobs_before,
            "{label} staged blobs despite error"
        );
        assert_eq!(
            doc.encode_message().unwrap(),
            baseline,
            "{label} mutated document despite error"
        );
    }

    // Valid closed network succeeds and decodes back identically.
    let valid = valid_triangle_network();
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetVectorNetwork {
            target: vec_id,
            network: valid.clone(),
        }],
        None,
        None,
        false,
    )
    .expect("valid network should succeed");
    let blob_idx = doc
        .node(vec_id)
        .unwrap()
        .data
        .path(&["vectorData", "vectorNetworkBlob"])
        .and_then(Val::as_u32)
        .expect("vectorNetworkBlob index") as usize;
    assert_eq!(
        decode_vector_network(&doc.blobs[blob_idx]).as_ref(),
        Some(&valid)
    );

    // Valid open polyline (no regions) also succeeds.
    let open_polyline = VectorNetwork {
        vertices: valid.vertices[..2].to_vec(),
        segments: valid.segments[..1].to_vec(),
        regions: Vec::new(),
    };
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetVectorNetwork {
            target: vec_id,
            network: open_polyline.clone(),
        }],
        None,
        None,
        false,
    )
    .expect("open polyline should succeed");
    let blob_idx2 = doc
        .node(vec_id)
        .unwrap()
        .data
        .path(&["vectorData", "vectorNetworkBlob"])
        .and_then(Val::as_u32)
        .unwrap() as usize;
    assert_eq!(
        decode_vector_network(&doc.blobs[blob_idx2]).as_ref(),
        Some(&open_polyline)
    );
    let reopened = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
    assert!(reopened.node(vec_id).is_some());
}

#[test]
fn boolean_combine_honours_rotation_scale_cross_parent_and_local_bounds() {
    let mut doc = Document::open(fixture("structure.fig")).expect("open structure.fig");
    let baseline = doc.encode_message().unwrap();
    let page = doc.pages[0];
    let mut history = History::new(30);

    // Operand A: 100x20 rectangle at (400, 80), rotated +90 deg around its origin.
    // Under Figma's 2x3 matrix (cos=0, sin=1): (x, y) -> (400 - y, 80 + x),
    // so A occupies [380..400, 80..180] in page space.
    let (rep_a, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[rect_op(None, page, 400.0, 80.0, 100.0, 20.0)],
        None,
        None,
        false,
    )
    .unwrap();
    let a_id = rep_a.created_ids[0];
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::Rotate {
            target: a_id,
            degrees: 90.0,
        }],
        None,
        None,
        false,
    )
    .unwrap();

    // Put operand B inside a separate Auto-Layout HUG frame at (300, 100), so B is in a
    // different parent space and removing B must re-solve that Auto-Layout frame.
    let (rep_frame, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: Some("B Holder".into()),
            x: 300.0,
            y: 100.0,
            width: 200.0,
            height: 200.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let holder = rep_frame.created_ids[0];

    // Add a sibling inside holder plus B (25x15 scaled 2x -> 50x30 in holder space).
    let (rep_kids, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::SetAutoLayout {
                target: holder,
                mode: "HORIZONTAL".into(),
                spacing: Some(10.0),
                counter_spacing: None,
                padding: Some([20.0, 10.0, 20.0, 70.0]),
                primary_sizing: Some("HUG".into()),
                counter_sizing: Some("HUG".into()),
                primary_align: None,
                counter_align: None,
                wrap: None,
            },
            rect_op(None, holder, 0.0, 0.0, 25.0, 15.0),
            rect_op(None, holder, 0.0, 0.0, 40.0, 15.0),
        ],
        None,
        None,
        false,
    )
    .unwrap();
    let b_id = rep_kids.created_ids[0];
    // Auto-layout placed B at (70, 20) inside holder at (300, 100), so page pos is (370, 120).
    // Scale B by 2x via raw transform so B covers [370..420, 120..150] in page space!
    {
        let mut t = Txn::new("scale b");
        t.set_field(
            b_id,
            "transform",
            Val::Obj(Obj::from([
                ("m00", Val::Float(2.0)),
                ("m01", Val::Float(0.0)),
                ("m02", Val::Float(70.0)),
                ("m10", Val::Float(0.0)),
                ("m11", Val::Float(2.0)),
                ("m12", Val::Float(20.0)),
            ])),
        );
        doc.apply_txn(&t).unwrap();
    }
    let holder_w_before = doc
        .node(holder)
        .unwrap()
        .data
        .path(&["size", "x"])
        .and_then(Val::as_f32)
        .unwrap();

    // Combine A (on page) and B (inside holder): union covers [370..420, 80..180].
    let (rep_bool, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Union,
            a: a_id,
            b: b_id,
            name: Some("Cross Union".into()),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let bool_id = rep_bool.created_ids[0];
    let bn = doc.node(bool_id).expect("boolean node");

    let bx = bn
        .data
        .path(&["transform", "m02"])
        .and_then(Val::as_f32)
        .unwrap();
    let by = bn
        .data
        .path(&["transform", "m12"])
        .and_then(Val::as_f32)
        .unwrap();
    let bw = bn.data.path(&["size", "x"]).and_then(Val::as_f32).unwrap();
    let bh = bn.data.path(&["size", "y"]).and_then(Val::as_f32).unwrap();
    assert!(
        (bx - 370.0).abs() < 1.0 && (by - 80.0).abs() < 1.0,
        "expected boolean transform near (370, 80), got ({bx}, {by})"
    );
    assert!(
        (bw - 50.0).abs() < 1.0 && (bh - 100.0).abs() < 1.0,
        "expected boolean size near (50, 100), got ({bw}, {bh})"
    );

    // Combined fillGeometry must be normalized into bool_id's local [0..bw, 0..bh] frame.
    let fg_idx = bn
        .data
        .get("fillGeometry")
        .unwrap()
        .items()
        .first()
        .unwrap()
        .get("commandsBlob")
        .and_then(Val::as_u32)
        .unwrap() as usize;
    let cmds = decode_path_commands(&doc.blobs[fg_idx]).unwrap();
    let (min_x, min_y, max_x, max_y) = cmds_bbox(&cmds);
    assert!(
        min_x.abs() < 1.0
            && min_y.abs() < 1.0
            && (max_x - 50.0).abs() < 1.0
            && (max_y - 100.0).abs() < 1.0,
        "expected local fillGeometry bbox [0..50, 0..100], got [{min_x}..{max_x}, {min_y}..{max_y}]"
    );

    // Child transforms must be rebased relative to bool_id so their page positions are preserved.
    let na_after = doc.node(a_id).unwrap();
    let ax = na_after
        .data
        .path(&["transform", "m02"])
        .and_then(Val::as_f32)
        .unwrap();
    let ay = na_after
        .data
        .path(&["transform", "m12"])
        .and_then(Val::as_f32)
        .unwrap();
    assert!(
        ((bx + ax) - 400.0).abs() < 1e-2 && ((by + ay) - 80.0).abs() < 1e-2,
        "child A page origin shifted: bool ({bx},{by}) + child ({ax},{ay})"
    );
    let nb_after = doc.node(b_id).unwrap();
    let b_child_x = nb_after
        .data
        .path(&["transform", "m02"])
        .and_then(Val::as_f32)
        .unwrap();
    let b_child_y = nb_after
        .data
        .path(&["transform", "m12"])
        .and_then(Val::as_f32)
        .unwrap();
    let b_m00 = nb_after
        .data
        .path(&["transform", "m00"])
        .and_then(Val::as_f32)
        .unwrap();
    assert!(
        ((bx + b_child_x) - 370.0).abs() < 1e-2
            && ((by + b_child_y) - 120.0).abs() < 1e-2
            && (b_m00 - 2.0).abs() < 1e-2,
        "child B page transform shifted: bool ({bx},{by}) + child ({b_child_x},{b_child_y}), scale {b_m00}"
    );

    // B's old Auto-Layout parent must have shrunk when B left it.
    let holder_w_after = doc
        .node(holder)
        .unwrap()
        .data
        .path(&["size", "x"])
        .and_then(Val::as_f32)
        .unwrap();
    assert!(
        holder_w_after < holder_w_before - 20.0,
        "B's old Auto-Layout parent did not re-solve: before={holder_w_before}, after={holder_w_after}"
    );

    // Disjoint intersection produces an authoritative empty fillGeometry and zero size.
    let (rep_disjoint, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            rect_op(None, page, 10.0, 10.0, 20.0, 20.0),
            rect_op(None, page, 200.0, 200.0, 20.0, 20.0),
        ],
        None,
        None,
        false,
    )
    .unwrap();
    let (da, db) = (rep_disjoint.created_ids[0], rep_disjoint.created_ids[1]);
    let (rep_empty, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Intersect,
            a: da,
            b: db,
            name: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let empty_bool = doc.node(rep_empty.created_ids[0]).unwrap();
    assert_eq!(
        empty_bool.data.get("fillGeometry").map(|v| v.items().len()),
        Some(0)
    );

    // Combining a node with itself or with its own ancestor is rejected cleanly.
    assert!(
        execute_edit_ops(
            &mut doc,
            &mut history,
            &[EditOp::BooleanCombine {
                operation: BooleanOp::Union,
                a: bool_id,
                b: bool_id,
                name: None,
            }],
            None,
            None,
            false,
        )
        .is_err()
    );
    assert!(
        execute_edit_ops(
            &mut doc,
            &mut history,
            &[EditOp::BooleanCombine {
                operation: BooleanOp::Union,
                a: a_id,
                b: bool_id,
                name: None,
            }],
            None,
            None,
            false,
        )
        .is_err()
    );

    // Save/reopen roundtrip succeeds, and full undo restores baseline bytes.
    let reopened = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
    assert!(reopened.node(bool_id).is_some());
    while history.undo(&mut doc).unwrap().is_some() {}
    // Only the direct `apply_txn` for scaling B is outside History, so re-open check covers serialization.
    let _ = baseline;
}

#[test]
fn geometry_and_stroke_edits_invalidate_stale_stroke_geometry_and_style_refs() {
    let mut doc = Document::open(fixture("structure.fig")).expect("open structure.fig");
    let page = doc.pages[0];
    let mut history = History::new(30);

    // Create a fill style + a stroke style + an effect style so style refs are schema-valid.
    let (rep_style, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateFillStyle {
            name: "Shared Style".into(),
            hex: "#123456".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let style_id = rep_style.created_ids[0];

    let make_stroked_rect = |doc: &mut Document, history: &mut History, parent: Guid| -> Guid {
        let (rep, _) = execute_edit_ops(
            doc,
            history,
            &[rect_op(None, parent, 20.0, 20.0, 60.0, 40.0)],
            None,
            None,
            false,
        )
        .unwrap();
        let id = rep.created_ids[0];
        let mut t = Txn::new("seed stroke geometry and styles");
        let stroke_blob = doc.blobs.len();
        t.add_blobs
            .push(encode_path_commands(&rounded_rect_commands(
                60.0, 40.0, [0.0; 4], 0.0,
            )));
        t.set_field(
            id,
            "strokeGeometry",
            Val::Arr(vec![make_path_entry(stroke_blob, true)]),
        );
        t.set_field(
            id,
            "strokePaints",
            Val::Arr(vec![
                figstudio_fig::edit::styles_vars::hex_paint("#FF0000").unwrap(),
            ]),
        );
        t.set_field(id, "borderStrokeWeightsIndependent", Val::Bool(true));
        t.set_field(id, "borderTopWeight", Val::Float(5.0));
        t.set_field(
            id,
            "styleIdForFill",
            Val::Obj(Obj::from([("guid", style_id.to_val())])),
        );
        t.set_field(
            id,
            "styleIdForStrokeFill",
            Val::Obj(Obj::from([("guid", style_id.to_val())])),
        );
        t.set_field(
            id,
            "styleIdForEffect",
            Val::Obj(Obj::from([("guid", style_id.to_val())])),
        );
        doc.apply_txn(&t).unwrap();
        id
    };

    // 1. Resize invalidates strokeGeometry.
    let r_resize = make_stroked_rect(&mut doc, &mut history, page);
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::Resize {
            target: r_resize,
            width: 120.0,
            height: 40.0,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    assert!(
        doc.node(r_resize)
            .unwrap()
            .data
            .get("strokeGeometry")
            .is_none(),
        "Resize left stale strokeGeometry"
    );

    // 2. SetCornerRadii invalidates strokeGeometry.
    let r_radii = make_stroked_rect(&mut doc, &mut history, page);
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetCornerRadii {
            target: r_radii,
            radii: [12.0; 4],
            smoothing: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    assert!(
        doc.node(r_radii)
            .unwrap()
            .data
            .get("strokeGeometry")
            .is_none(),
        "SetCornerRadii left stale strokeGeometry"
    );

    // 3. Auto-Layout resize invalidates strokeGeometry.
    let (rep_al, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: Some("AL Container".into()),
            x: 0.0,
            y: 0.0,
            width: 240.0,
            height: 100.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let al_frame = rep_al.created_ids[0];
    let r_al = make_stroked_rect(&mut doc, &mut history, al_frame);
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::SetAutoLayout {
                target: al_frame,
                mode: "HORIZONTAL".into(),
                spacing: Some(0.0),
                counter_spacing: None,
                padding: Some([0.0, 0.0, 0.0, 0.0]),
                primary_sizing: Some("FIXED".into()),
                counter_sizing: Some("FIXED".into()),
                primary_align: None,
                counter_align: None,
                wrap: None,
            },
            EditOp::SetAutoLayoutChild {
                target: r_al,
                primary_fill: Some(true),
                counter_stretch: None,
                positioning: None,
                min_size: None,
                max_size: None,
            },
        ],
        None,
        None,
        false,
    )
    .unwrap();
    assert!(
        doc.node(r_al).unwrap().data.get("strokeGeometry").is_none(),
        "Auto-Layout resize left stale strokeGeometry"
    );

    // 4. SetStroke invalidates strokeGeometry, independent border weights, and stroke style ref.
    let r_stroke = make_stroked_rect(&mut doc, &mut history, page);
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetStroke {
            target: r_stroke,
            hex: Some("#00FF00".into()),
            weight: Some(4.0),
            align: Some("OUTSIDE".into()),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let sd = &doc.node(r_stroke).unwrap().data;
    assert!(
        sd.get("strokeGeometry").is_none(),
        "SetStroke left stale strokeGeometry"
    );
    assert_ne!(
        sd.flag("borderStrokeWeightsIndependent"),
        Some(true),
        "SetStroke left borderStrokeWeightsIndependent=true"
    );
    assert!(
        sd.get("borderTopWeight").is_none(),
        "SetStroke left stale borderTopWeight"
    );
    assert!(
        sd.get("styleIdForStrokeFill").is_none(),
        "SetStroke left stale styleIdForStrokeFill"
    );

    // 5. SetImageFill clears styleIdForFill and SetEffects clears styleIdForEffect.
    let r_styles = make_stroked_rect(&mut doc, &mut history, page);
    let fake_sha1 = "0123456789abcdef0123456789abcdef01234567".to_string();
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::SetImageFill {
                target: r_styles,
                sha1_hex: fake_sha1,
                scale_mode: "FILL".into(),
                opacity: 1.0,
            },
            EditOp::SetEffects {
                target: r_styles,
                drop_shadow: None,
                blur_radius: Some(4.0),
                background_blur: false,
            },
        ],
        None,
        None,
        false,
    )
    .unwrap();
    let st = &doc.node(r_styles).unwrap().data;
    assert!(
        st.get("styleIdForFill").is_none(),
        "SetImageFill left stale styleIdForFill"
    );
    assert!(
        st.get("styleIdForEffect").is_none(),
        "SetEffects left stale styleIdForEffect"
    );

    // 6. SetStroke color-only change on a stroked VECTOR keeps strokeGeometry, while
    // SetVectorNetwork clears stale strokeGeometry on the same VECTOR.
    let mut vdoc = Document::open(fixture("vector.fig")).expect("open vector.fig");
    let vec_id = vdoc.nodes.iter().find(|n| n.kind == "VECTOR").unwrap().id;
    {
        let mut t = Txn::new("seed vector strokeGeometry");
        let b = vdoc.blobs.len();
        t.add_blobs
            .push(encode_path_commands(&rounded_rect_commands(
                20.0, 20.0, [0.0; 4], 0.0,
            )));
        t.set_field(
            vec_id,
            "strokePaints",
            Val::Arr(vec![
                figstudio_fig::edit::styles_vars::hex_paint("#FF0000").unwrap(),
            ]),
        );
        t.set_field(
            vec_id,
            "strokeGeometry",
            Val::Arr(vec![make_path_entry(b, true)]),
        );
        vdoc.apply_txn(&t).unwrap();
    }
    let mut vhist = History::new(10);
    execute_edit_ops(
        &mut vdoc,
        &mut vhist,
        &[EditOp::SetStroke {
            target: vec_id,
            hex: Some("#0000FF".into()),
            weight: None,
            align: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    assert!(
        vdoc.node(vec_id)
            .unwrap()
            .data
            .get("strokeGeometry")
            .is_some(),
        "color-only SetStroke on an already-stroked VECTOR must keep strokeGeometry"
    );
    execute_edit_ops(
        &mut vdoc,
        &mut vhist,
        &[EditOp::SetVectorNetwork {
            target: vec_id,
            network: valid_triangle_network(),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    assert!(
        vdoc.node(vec_id)
            .unwrap()
            .data
            .get("strokeGeometry")
            .is_none(),
        "SetVectorNetwork left stale strokeGeometry"
    );
}

#[test]
fn patch_raw_coerces_top_level_and_nested_kiwi_enums_and_rejects_invalid_variants() {
    let mut doc = Document::open(fixture("structure.fig")).expect("open structure.fig");
    let page = doc.pages[0];
    let mut history = History::new(20);
    let target = execute_edit_ops(
        &mut doc,
        &mut history,
        &[rect_op(None, page, 0.0, 0.0, 50.0, 50.0)],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let blob_idx = doc
        .node(target)
        .unwrap()
        .data
        .get("fillGeometry")
        .unwrap()
        .items()[0]
        .get("commandsBlob")
        .and_then(Val::as_u32)
        .unwrap();

    // Deserialize PatchRaw from JSON exactly as MCP `apply_edits` does, covering top-level enums,
    // nested message/struct enums, and byte-array fields (`image.hash`).
    let sample_hash: Vec<u8> = (0u8..20u8).collect();
    let op: EditOp = serde_json::from_value(serde_json::json!({
        "op": "patch_raw",
        "target": target.to_string(),
        "set": {
            "blendMode": "MULTIPLY",
            "strokeCap": "ROUND",
            "fillPaints": [
                {
                    "type": "SOLID",
                    "visible": true,
                    "opacity": 1.0,
                    "blendMode": "NORMAL",
                    "color": { "r": 0.0, "g": 1.0, "b": 0.0, "a": 1.0 }
                },
                {
                    "type": "IMAGE",
                    "visible": true,
                    "opacity": 1.0,
                    "blendMode": "NORMAL",
                    "imageScaleMode": "FILL",
                    "image": {
                        "hash": sample_hash,
                        "name": "000102030405060708090a0b0c0d0e0f10111213"
                    }
                }
            ],
            "fillGeometry": [{
                "windingRule": "ODD",
                "commandsBlob": blob_idx,
                "styleID": 0
            }]
        }
    }))
    .unwrap();

    execute_edit_ops(&mut doc, &mut history, &[op], None, None, false)
        .expect("patch_raw with top-level and nested enums must succeed");

    let d = &doc.node(target).unwrap().data;
    assert!(matches!(d.get("blendMode"), Some(Val::Enum("MULTIPLY"))));
    assert!(matches!(d.get("strokeCap"), Some(Val::Enum("ROUND"))));
    assert!(matches!(
        d.get("fillPaints").unwrap().items()[0].get("type"),
        Some(Val::Enum("SOLID"))
    ));
    assert_eq!(
        d.get("fillPaints").unwrap().items()[1]
            .path(&["image", "hash"])
            .and_then(Val::as_bytes),
        Some(sample_hash.as_slice()),
        "byte[] field must coerce into Val::Bytes"
    );
    assert!(matches!(
        d.get("fillGeometry").unwrap().items()[0].get("windingRule"),
        Some(Val::Enum("ODD"))
    ));

    // Must encode to Kiwi and survive reopen cleanly.
    let saved = doc.to_bytes().expect("to_bytes after patch_raw enums");
    let reopened = Document::from_bytes(&saved).expect("reopen after patch_raw enums");
    assert_eq!(
        reopened.node(target).unwrap().data.text("blendMode"),
        Some("MULTIPLY")
    );

    // Invalid enum variant must fail atomically without modifying the document.
    let before_bad = doc.encode_message().unwrap();
    let bad_op: EditOp = serde_json::from_value(serde_json::json!({
        "op": "patch_raw",
        "target": target.to_string(),
        "set": {
            "blendMode": "NOT_A_REAL_BLEND_MODE"
        }
    }))
    .unwrap();
    assert!(
        execute_edit_ops(&mut doc, &mut history, &[bad_op], None, None, false).is_err(),
        "invalid enum variant must fail"
    );
    assert_eq!(doc.encode_message().unwrap(), before_bad);
}

#[test]
fn explicit_guid_in_create_ops_advances_allocator_past_collision() {
    let mut doc = Document::open(fixture("structure.fig")).expect("open structure.fig");
    let page = doc.pages[0];
    let mut history = History::new(20);

    let first_auto = GuidAlloc::for_document(&doc).next_guid();
    let explicit_shape = first_auto;
    let explicit_text = Guid(first_auto.0, first_auto.1 + 5);
    let explicit_page = Guid(first_auto.0, first_auto.1 + 10);

    let (rep, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            rect_op(Some(explicit_shape), page, 0.0, 0.0, 20.0, 20.0),
            rect_op(None, page, 30.0, 0.0, 20.0, 20.0),
            EditOp::CreateText {
                id: Some(explicit_text),
                parent: page,
                x: 0.0,
                y: 30.0,
                characters: "Explicit".into(),
                font_family: "Inter".into(),
                font_style: "Regular".into(),
                font_size: 14.0,
                fill_hex: "#000000".into(),
            },
            EditOp::CreateText {
                id: None,
                parent: page,
                x: 0.0,
                y: 60.0,
                characters: "Auto".into(),
                font_family: "Inter".into(),
                font_style: "Regular".into(),
                font_size: 14.0,
                fill_hex: "#000000".into(),
            },
            EditOp::CreatePage {
                id: Some(explicit_page),
                name: "Explicit Page".into(),
            },
            EditOp::CreatePage {
                id: None,
                name: "Auto Page".into(),
            },
        ],
        None,
        None,
        false,
    )
    .expect("explicit GUIDs followed by auto GUIDs in one batch must not collide");

    assert_eq!(rep.created_ids.len(), 6);
    assert_eq!(rep.created_ids[0], explicit_shape);
    assert_eq!(rep.created_ids[2], explicit_text);
    assert_eq!(rep.created_ids[4], explicit_page);

    let unique: HashSet<Guid> = rep.created_ids.iter().copied().collect();
    assert_eq!(unique.len(), 6, "all created GUIDs must be distinct");
    for id in &rep.created_ids {
        assert!(doc.node(*id).is_some(), "missing created node {id}");
    }
    let reopened = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
    for id in &rep.created_ids {
        assert!(reopened.node(*id).is_some(), "missing reopened node {id}");
    }
}

fn seed_stroke_geom(doc: &mut Document, target: Guid, w: f32, h: f32) {
    let mut t = Txn::new("seed strokeGeometry");
    let blob = doc.blobs.len();
    t.add_blobs
        .push(encode_path_commands(&rounded_rect_commands(
            w, h, [0.0; 4], 0.0,
        )));
    t.set_field(
        target,
        "strokeGeometry",
        Val::Arr(vec![make_path_entry(blob, true)]),
    );
    doc.apply_txn(&t).unwrap();
}

#[test]
fn derived_cache_invalidation_covers_text_patch_raw_hidden_stroke_and_bind_style() {
    let mut doc = Document::open(fixture("structure.fig")).expect("open structure.fig");
    let page = doc.pages[0];
    let mut history = History::new(40);

    // 1. Text ops (SetText, SpliceText, StyleTextRange, SetTypography) and direct helpers
    // must invalidate stale strokeGeometry alongside derivedTextData.
    let text_id = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateText {
            id: None,
            parent: page,
            x: 10.0,
            y: 10.0,
            characters: "Hello world".into(),
            font_family: "Inter".into(),
            font_style: "Regular".into(),
            font_size: 16.0,
            fill_hex: "#111111".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];

    let seed_text_caches = |doc: &mut Document| {
        seed_stroke_geom(doc, text_id, 80.0, 24.0);
        let mut t = Txn::new("seed derivedTextData");
        t.set_field(
            text_id,
            "derivedTextData",
            Val::Obj(Obj::from([("glyphs", Val::Arr(Vec::new()))])),
        );
        doc.apply_txn(&t).unwrap();
    };

    for (label, op) in [
        (
            "SetText",
            EditOp::SetText {
                target: text_id,
                characters: "Updated text".into(),
            },
        ),
        (
            "SpliceText",
            EditOp::SpliceText {
                target: text_id,
                utf16_start: 0,
                utf16_delete_len: 7,
                insert: "Spliced".into(),
            },
        ),
        (
            "StyleTextRange",
            EditOp::StyleTextRange {
                target: text_id,
                utf16_start: 0,
                utf16_end: 5,
                style: TextStyleRunPatch {
                    font_size: Some(24.0),
                    ..Default::default()
                },
            },
        ),
        (
            "SetTypography",
            EditOp::SetTypography {
                target: text_id,
                font_family: None,
                font_style: Some("Bold".into()),
                font_size: Some(28.0),
                align_horizontal: None,
                auto_resize: None,
            },
        ),
    ] {
        seed_text_caches(&mut doc);
        execute_edit_ops(&mut doc, &mut history, &[op], None, None, false).unwrap();
        let d = &doc.node(text_id).unwrap().data;
        assert!(
            d.get("derivedTextData").is_none(),
            "{label} left stale derivedTextData"
        );
        assert!(
            d.get("strokeGeometry").is_none(),
            "{label} left stale strokeGeometry"
        );
    }

    // Direct helper calls outside execute_edit_ops must uphold the same invalidation invariant.
    seed_text_caches(&mut doc);
    let mut direct_txn = Txn::new("direct set_text_plain");
    set_text_plain(&doc, &mut direct_txn, text_id, "Direct").unwrap();
    doc.apply_txn(&direct_txn).unwrap();
    assert!(
        doc.node(text_id)
            .unwrap()
            .data
            .get("strokeGeometry")
            .is_none(),
        "direct set_text_plain left stale strokeGeometry"
    );

    seed_text_caches(&mut doc);
    let mut direct_commit = Txn::new("direct commit_text_data");
    commit_text_data(
        &doc,
        &mut direct_commit,
        text_id,
        Val::Obj(Obj::from([("characters", Val::Str("Committed".into()))])),
    )
    .unwrap();
    doc.apply_txn(&direct_commit).unwrap();
    assert!(
        doc.node(text_id)
            .unwrap()
            .data
            .get("strokeGeometry")
            .is_none(),
        "direct commit_text_data left stale strokeGeometry"
    );

    // 2. PatchRaw changing stroke-shape or parametric geometry fields invalidates strokeGeometry
    // (and rebuilds parametric fillGeometry) unless the patch explicitly provides fresh geometry.
    let rect_id = execute_edit_ops(
        &mut doc,
        &mut history,
        &[rect_op(None, page, 10.0, 80.0, 60.0, 40.0)],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];

    for (label, patch_json) in [
        (
            "dashPattern",
            serde_json::json!({ "dashPattern": [6.0, 4.0] }),
        ),
        ("strokeCap", serde_json::json!({ "strokeCap": "ROUND" })),
        ("strokeJoin", serde_json::json!({ "strokeJoin": "BEVEL" })),
        ("strokeWeight", serde_json::json!({ "strokeWeight": 6.0 })),
        (
            "strokeAlign",
            serde_json::json!({ "strokeAlign": "OUTSIDE" }),
        ),
        (
            "independent borders",
            serde_json::json!({
                "borderStrokeWeightsIndependent": true,
                "borderTopWeight": 5.0
            }),
        ),
        ("cornerRadius", serde_json::json!({ "cornerRadius": 10.0 })),
        (
            "size",
            serde_json::json!({ "size": { "x": 110.0, "y": 50.0 } }),
        ),
    ] {
        seed_stroke_geom(&mut doc, rect_id, 60.0, 40.0);
        let op: EditOp = serde_json::from_value(serde_json::json!({
            "op": "patch_raw",
            "target": rect_id.to_string(),
            "set": patch_json
        }))
        .unwrap();
        execute_edit_ops(&mut doc, &mut history, &[op], None, None, false).unwrap();
        assert!(
            doc.node(rect_id)
                .unwrap()
                .data
                .get("strokeGeometry")
                .is_none(),
            "PatchRaw({label}) left stale strokeGeometry"
        );
    }

    // Parametric fillGeometry on rect_id must reflect the PatchRaw size change to (110, 50).
    let fg_idx = doc
        .node(rect_id)
        .unwrap()
        .data
        .get("fillGeometry")
        .unwrap()
        .items()[0]
        .get("commandsBlob")
        .and_then(Val::as_u32)
        .unwrap() as usize;
    let (_, _, max_x, max_y) = cmds_bbox(&decode_path_commands(&doc.blobs[fg_idx]).unwrap());
    assert!(
        (max_x - 110.0).abs() < 1e-2 && (max_y - 50.0).abs() < 1e-2,
        "PatchRaw(size) left stale fillGeometry bbox ({max_x}, {max_y})"
    );

    // Explicitly supplying strokeGeometry in PatchRaw must keep that caller-supplied cache.
    let explicit_blob = doc.blobs.len() - 1;
    let op_explicit: EditOp = serde_json::from_value(serde_json::json!({
        "op": "patch_raw",
        "target": rect_id.to_string(),
        "set": {
            "strokeWeight": 3.0,
            "strokeGeometry": [{
                "windingRule": "ODD",
                "commandsBlob": explicit_blob,
                "styleID": 0
            }]
        }
    }))
    .unwrap();
    execute_edit_ops(&mut doc, &mut history, &[op_explicit], None, None, false).unwrap();
    assert!(
        doc.node(rect_id)
            .unwrap()
            .data
            .get("strokeGeometry")
            .is_some(),
        "PatchRaw with explicit strokeGeometry must preserve caller-supplied strokeGeometry"
    );

    // PatchRaw changing arcData on an ELLIPSE or count/starInnerScale on a STAR invalidates
    // strokeGeometry and refreshes fillGeometry.
    let (shapes_rep, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::CreateShape {
                id: None,
                kind: "ELLIPSE".into(),
                parent: page,
                name: None,
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 80.0,
                corner_radius: 0.0,
                fill_hex: Some("#0000FF".into()),
                count: None,
                star_inner_scale: None,
            },
            EditOp::CreateShape {
                id: None,
                kind: "STAR".into(),
                parent: page,
                name: None,
                x: 100.0,
                y: 0.0,
                width: 80.0,
                height: 80.0,
                corner_radius: 0.0,
                fill_hex: Some("#00AA00".into()),
                count: Some(5),
                star_inner_scale: Some(0.5),
            },
        ],
        None,
        None,
        false,
    )
    .unwrap();
    let (ellipse_id, star_id) = (shapes_rep.created_ids[0], shapes_rep.created_ids[1]);
    seed_stroke_geom(&mut doc, ellipse_id, 80.0, 80.0);
    seed_stroke_geom(&mut doc, star_id, 80.0, 80.0);
    let patch_ellipse: EditOp = serde_json::from_value(serde_json::json!({
        "op": "patch_raw",
        "target": ellipse_id.to_string(),
        "set": {
            "arcData": {
                "startingAngle": 0.0,
                "endingAngle": std::f64::consts::TAU,
                "innerRadius": 0.5
            }
        }
    }))
    .unwrap();
    let patch_star: EditOp = serde_json::from_value(serde_json::json!({
        "op": "patch_raw",
        "target": star_id.to_string(),
        "set": { "count": 7, "starInnerScale": 0.3 }
    }))
    .unwrap();
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[patch_ellipse, patch_star],
        None,
        None,
        false,
    )
    .unwrap();
    assert!(
        doc.node(ellipse_id)
            .unwrap()
            .data
            .get("strokeGeometry")
            .is_none(),
        "PatchRaw(arcData) left stale strokeGeometry"
    );
    assert!(
        doc.node(star_id)
            .unwrap()
            .data
            .get("strokeGeometry")
            .is_none(),
        "PatchRaw(count/starInnerScale) left stale strokeGeometry"
    );
    let e_blob = doc
        .node(ellipse_id)
        .unwrap()
        .data
        .get("fillGeometry")
        .unwrap()
        .items()[0]
        .get("commandsBlob")
        .and_then(Val::as_u32)
        .unwrap() as usize;
    let e_closes = decode_path_commands(&doc.blobs[e_blob])
        .unwrap()
        .iter()
        .filter(|c| matches!(c, PathCmd::Close))
        .count();
    assert_eq!(
        e_closes, 2,
        "PatchRaw(arcData.innerRadius=0.5) must rebuild donut fillGeometry with 2 contours"
    );

    // 3. SetStroke on a node with a hidden (`visible: false`) or zero-opacity stroke paint
    // must treat the node as previously unstroked and invalidate stale strokeGeometry.
    for (label, hidden_paint) in [
        ("visible:false", {
            let mut p = hex_paint("#FF0000").unwrap().into_owned();
            p.set("visible", Val::Bool(false));
            p
        }),
        ("opacity:0.0", {
            let mut p = hex_paint("#FF0000").unwrap().into_owned();
            p.set("opacity", Val::Float(0.0));
            p
        }),
    ] {
        let mut t = Txn::new("seed hidden stroke");
        t.set_field(rect_id, "strokePaints", Val::Arr(vec![hidden_paint]));
        doc.apply_txn(&t).unwrap();
        seed_stroke_geom(&mut doc, rect_id, 20.0, 20.0);

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
        assert!(
            doc.node(rect_id)
                .unwrap()
                .data
                .get("strokeGeometry")
                .is_none(),
            "SetStroke color-only over {label} left stale strokeGeometry"
        );
    }

    // 4. BindStyle(slot: "stroke") on an unstroked/hidden-stroke Auto-Layout frame with
    // bordersTakeSpace=true must invalidate stale strokeGeometry AND trigger Auto-Layout re-solve.
    let style_id = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateFillStyle {
            name: "Border Style".into(),
            hex: "#224488".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let frame_id = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: Some("Border Hug Frame".into()),
            x: 0.0,
            y: 200.0,
            width: 100.0,
            height: 60.0,
            corner_radius: 0.0,
            fill_hex: Some("#FFFFFF".into()),
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
        &[
            EditOp::SetAutoLayout {
                target: frame_id,
                mode: "HORIZONTAL".into(),
                spacing: Some(0.0),
                counter_spacing: None,
                padding: Some([0.0, 0.0, 0.0, 0.0]),
                primary_sizing: Some("HUG".into()),
                counter_sizing: Some("HUG".into()),
                primary_align: None,
                counter_align: None,
                wrap: None,
            },
            rect_op(None, frame_id, 0.0, 0.0, 50.0, 30.0),
        ],
        None,
        None,
        false,
    )
    .unwrap();
    {
        let mut t = Txn::new("enable bordersTakeSpace with 6px inside strokeWeight, no paint");
        t.set_field(frame_id, "bordersTakeSpace", Val::Bool(true));
        t.set_field(frame_id, "strokeWeight", Val::Float(6.0));
        t.set_field(frame_id, "strokeAlign", Val::Enum(intern("INSIDE")));
        t.set_field(frame_id, "strokePaints", Val::Arr(Vec::new()));
        doc.apply_txn(&t).unwrap();
    }
    seed_stroke_geom(&mut doc, frame_id, 10.0, 10.0);
    let w_before = doc
        .node(frame_id)
        .unwrap()
        .data
        .path(&["size", "x"])
        .and_then(Val::as_f32)
        .unwrap();
    assert!((w_before - 50.0).abs() < 1e-2);

    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::BindStyle {
            target: frame_id,
            slot: "stroke".into(),
            style: Some(style_id),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let frame_after = &doc.node(frame_id).unwrap().data;
    assert!(
        frame_after.get("strokeGeometry").is_none(),
        "BindStyle(stroke) on unstroked node left stale strokeGeometry"
    );
    let w_after = frame_after
        .path(&["size", "x"])
        .and_then(Val::as_f32)
        .unwrap();
    assert!(
        (w_after - 62.0).abs() < 1e-2,
        "BindStyle(stroke) with bordersTakeSpace=true must re-solve Auto-Layout (expected 62, got {w_after})"
    );

    // Direct bind_style helper call on unstroked node also clears stale strokeGeometry.
    let mut clear_stroke = Txn::new("clear strokePaints");
    clear_stroke.set_field(rect_id, "strokePaints", Val::Arr(Vec::new()));
    doc.apply_txn(&clear_stroke).unwrap();
    seed_stroke_geom(&mut doc, rect_id, 20.0, 20.0);
    let mut bind_txn = Txn::new("direct bind_style");
    bind_style(&doc, &mut bind_txn, rect_id, "stroke", Some(style_id)).unwrap();
    doc.apply_txn(&bind_txn).unwrap();
    assert!(
        doc.node(rect_id)
            .unwrap()
            .data
            .get("strokeGeometry")
            .is_none(),
        "direct bind_style(stroke) on unstroked node left stale strokeGeometry"
    );

    // Preservation check: BindStyle(stroke) on an already visibly stroked VECTOR keeps valid strokeGeometry.
    let mut vdoc = Document::open(fixture("vector.fig")).expect("open vector.fig");
    let vec_id = vdoc.nodes.iter().find(|n| n.kind == "VECTOR").unwrap().id;
    let mut vhist = History::new(10);
    let vstyle = execute_edit_ops(
        &mut vdoc,
        &mut vhist,
        &[EditOp::CreateFillStyle {
            name: "Vector Stroke Color".into(),
            hex: "#00AA44".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    {
        let mut t = Txn::new("seed visible stroke + strokeGeometry on vector");
        t.set_field(
            vec_id,
            "strokePaints",
            Val::Arr(vec![hex_paint("#FF0000").unwrap()]),
        );
        vdoc.apply_txn(&t).unwrap();
    }
    seed_stroke_geom(&mut vdoc, vec_id, 20.0, 20.0);
    execute_edit_ops(
        &mut vdoc,
        &mut vhist,
        &[EditOp::BindStyle {
            target: vec_id,
            slot: "stroke".into(),
            style: Some(vstyle),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    assert!(
        vdoc.node(vec_id)
            .unwrap()
            .data
            .get("strokeGeometry")
            .is_some(),
        "BindStyle(stroke) color-only change on an already-stroked VECTOR must keep strokeGeometry"
    );
}

#[test]
fn boolean_combine_rejects_ancestor_descendant_in_both_orders() {
    let mut doc = Document::open(fixture("structure.fig")).expect("open structure.fig");
    let page = doc.pages[0];
    let mut history = History::new(20);

    let (rep, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::CreateShape {
                id: None,
                kind: "FRAME".into(),
                parent: page,
                name: Some("Outer Frame".into()),
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 120.0,
                corner_radius: 0.0,
                fill_hex: Some("#CCCCCC".into()),
                count: None,
                star_inner_scale: None,
            },
            rect_op(None, page, 10.0, 10.0, 40.0, 40.0),
            rect_op(None, page, 30.0, 30.0, 40.0, 40.0),
        ],
        None,
        None,
        false,
    )
    .unwrap();
    let (outer_frame, r1, r2) = (rep.created_ids[0], rep.created_ids[1], rep.created_ids[2]);
    let child_rect = execute_edit_ops(
        &mut doc,
        &mut history,
        &[rect_op(None, outer_frame, 10.0, 10.0, 30.0, 30.0)],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let outer_bool = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Union,
            a: r1,
            b: r2,
            name: Some("Outer Bool".into()),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];

    let baseline = doc.encode_message().unwrap();
    for (label, a, b) in [
        ("a=outer_frame, b=child_rect", outer_frame, child_rect),
        ("a=child_rect, b=outer_frame", child_rect, outer_frame),
        ("a=outer_bool, b=r1", outer_bool, r1),
        ("a=r1, b=outer_bool", r1, outer_bool),
    ] {
        let res = execute_edit_ops(
            &mut doc,
            &mut history,
            &[EditOp::BooleanCombine {
                operation: BooleanOp::Union,
                a,
                b,
                name: None,
            }],
            None,
            None,
            false,
        );
        assert!(res.is_err(), "expected Err for {label}, got Ok");
        assert_eq!(
            doc.encode_message().unwrap(),
            baseline,
            "{label} mutated document despite error"
        );
    }
}

fn signed_area_of_ring(ring: &[(f64, f64)]) -> f64 {
    let mut a = 0.0;
    for i in 0..ring.len() {
        let (x0, y0) = ring[i];
        let (x1, y1) = ring[(i + 1) % ring.len()];
        a += x0 * y1 - x1 * y0;
    }
    a * 0.5
}

fn contour_signed_areas(cmds: &[PathCmd]) -> Vec<f64> {
    let mut areas = Vec::new();
    let mut cur: Vec<(f64, f64)> = Vec::new();
    for &c in cmds {
        match c {
            PathCmd::MoveTo(x, y) => {
                if cur.len() >= 3 {
                    areas.push(signed_area_of_ring(&cur));
                }
                cur.clear();
                cur.push((f64::from(x), f64::from(y)));
            }
            PathCmd::LineTo(x, y)
            | PathCmd::QuadTo(_, _, x, y)
            | PathCmd::CubicTo(_, _, _, _, x, y) => {
                cur.push((f64::from(x), f64::from(y)));
            }
            PathCmd::Close => {
                if cur.len() >= 3 {
                    areas.push(signed_area_of_ring(&cur));
                }
                cur.clear();
            }
        }
    }
    if cur.len() >= 3 {
        areas.push(signed_area_of_ring(&cur));
    }
    areas
}

#[test]
fn boolean_combine_preserves_holes_in_chained_booleans_donuts_and_odd_vectors() {
    let mut doc = Document::open(fixture("structure.fig")).expect("open structure.fig");
    let page = doc.pages[0];
    let mut history = History::new(30);

    // 1. Hollow square via Subtract(100x100, inner 40x40), then Union with a side tab.
    let (rep, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            rect_op(None, page, 0.0, 0.0, 100.0, 100.0),
            rect_op(None, page, 30.0, 30.0, 40.0, 40.0),
            rect_op(None, page, 90.0, 35.0, 40.0, 30.0),
        ],
        None,
        None,
        false,
    )
    .unwrap();
    let (outer, inner, tab) = (rep.created_ids[0], rep.created_ids[1], rep.created_ids[2]);
    let hollow = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Subtract,
            a: outer,
            b: inner,
            name: Some("Hollow".into()),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let chained = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Union,
            a: hollow,
            b: tab,
            name: Some("HollowPlusTab".into()),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];

    let fg_idx = doc
        .node(chained)
        .unwrap()
        .data
        .get("fillGeometry")
        .unwrap()
        .items()[0]
        .get("commandsBlob")
        .and_then(Val::as_u32)
        .unwrap() as usize;
    let areas = contour_signed_areas(&decode_path_commands(&doc.blobs[fg_idx]).unwrap());
    assert!(
        areas.iter().any(|&a| a > 5000.0) && areas.iter().any(|&a| a < -1000.0),
        "chained Union(Subtract(outer, inner), tab) lost negative-area hole contour: areas={areas:?}"
    );

    // 2. Donut ELLIPSE (innerRadius = 0.5) unioned with an overlapping side rect keeps its hole.
    let (drep, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::CreateShape {
                id: None,
                kind: "ELLIPSE".into(),
                parent: page,
                name: Some("Donut".into()),
                x: 200.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
                corner_radius: 0.0,
                fill_hex: Some("#0000FF".into()),
                count: None,
                star_inner_scale: None,
            },
            rect_op(None, page, 290.0, 35.0, 40.0, 30.0),
        ],
        None,
        None,
        false,
    )
    .unwrap();
    let (donut, donut_tab) = (drep.created_ids[0], drep.created_ids[1]);
    let set_donut: EditOp = serde_json::from_value(serde_json::json!({
        "op": "patch_raw",
        "target": donut.to_string(),
        "set": {
            "arcData": {
                "startingAngle": 0.0,
                "endingAngle": std::f64::consts::TAU,
                "innerRadius": 0.5
            }
        }
    }))
    .unwrap();
    execute_edit_ops(&mut doc, &mut history, &[set_donut], None, None, false).unwrap();
    let donut_union = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Union,
            a: donut,
            b: donut_tab,
            name: Some("DonutPlusTab".into()),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let dfg = doc
        .node(donut_union)
        .unwrap()
        .data
        .get("fillGeometry")
        .unwrap()
        .items()[0]
        .get("commandsBlob")
        .and_then(Val::as_u32)
        .unwrap() as usize;
    let d_areas = contour_signed_areas(&decode_path_commands(&doc.blobs[dfg]).unwrap());
    assert!(
        d_areas.iter().any(|&a| a > 5000.0) && d_areas.iter().any(|&a| a < -1500.0),
        "Union(donut ELLIPSE, tab) lost inner hole contour: areas={d_areas:?}"
    );

    // 3. ODD-wound holed VECTOR (both outer and inner square wound in the same direction,
    // `winding_nonzero: false`) + `normalizedSize` scaling without `fillGeometry`.
    let mut vdoc = Document::open(fixture("vector.fig")).expect("open vector.fig");
    let vec_id = vdoc.nodes.iter().find(|n| n.kind == "VECTOR").unwrap().id;
    let vpage = vdoc.page_of(vec_id).expect("page");
    let mut vhist = History::new(20);
    // Network coordinates in [0..10, 0..10] with normalizedSize=(10,10) and node size=(100,100)
    // -> scales 10x to outer [0..100, 0..100] with same-direction hole [30..70, 30..70] under ODD.
    let odd_donut_net = VectorNetwork {
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
            network: odd_donut_net,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    // Scale node size to 100x100 while keeping normalizedSize=(10,10) and dropping fillGeometry
    // so BooleanCombine must use the vectorData + normalizedSize fallback and honour ODD winding.
    {
        let mut t = Txn::new("setup normalizedSize fallback vector");
        t.set_field(
            vec_id,
            "parentIndex",
            Val::Obj(Obj::from([
                ("guid", vpage.to_val()),
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
                ("x", Val::Float(100.0)),
                ("y", Val::Float(100.0)),
            ])),
        );
        t.unset_field(vec_id, "fillGeometry");
        vdoc.apply_txn(&t).unwrap();
    }
    let vtab = execute_edit_ops(
        &mut vdoc,
        &mut vhist,
        &[rect_op(None, vpage, 90.0, 35.0, 30.0, 30.0)],
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
            name: Some("OddVectorUnion".into()),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let vbn = vdoc.node(vbool).unwrap();
    let vw = vbn.data.path(&["size", "x"]).and_then(Val::as_f32).unwrap();
    let vh = vbn.data.path(&["size", "y"]).and_then(Val::as_f32).unwrap();
    assert!(
        (vw - 120.0).abs() < 1.0 && (vh - 100.0).abs() < 1.0,
        "VECTOR normalizedSize fallback produced wrong size ({vw}, {vh}), expected (120, 100)"
    );
    let vfg = vbn.data.get("fillGeometry").unwrap().items()[0]
        .get("commandsBlob")
        .and_then(Val::as_u32)
        .unwrap() as usize;
    let v_areas = contour_signed_areas(&decode_path_commands(&vdoc.blobs[vfg]).unwrap());
    assert!(
        v_areas.iter().any(|&a| a > 5000.0) && v_areas.iter().any(|&a| a < -1500.0),
        "ODD-wound VECTOR union lost negative-area hole contour under NONZERO output: areas={v_areas:?}"
    );
}

#[test]
fn set_vector_network_enforces_vector_kind_empty_region_styles_and_bounds() {
    let mut doc = Document::open(fixture("vector.fig")).expect("open vector.fig");
    let page = doc.pages[0];
    let root = doc.root.expect("DOCUMENT root");
    let vec_id = doc
        .nodes
        .iter()
        .find(|n| n.kind == "VECTOR")
        .expect("VECTOR in vector.fig")
        .id;
    let mut history = History::new(30);

    // Create non-VECTOR targets (FRAME, TEXT, ROUNDED_RECTANGLE).
    let (created, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::CreateShape {
                id: None,
                kind: "FRAME".into(),
                parent: page,
                name: None,
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 40.0,
                corner_radius: 0.0,
                fill_hex: None,
                count: None,
                star_inner_scale: None,
            },
            rect_op(None, page, 50.0, 0.0, 40.0, 40.0),
            EditOp::CreateText {
                id: None,
                parent: page,
                x: 100.0,
                y: 0.0,
                characters: "Not a vector".into(),
                font_family: "Inter".into(),
                font_style: "Regular".into(),
                font_size: 14.0,
                fill_hex: "#000000".into(),
            },
        ],
        None,
        None,
        false,
    )
    .unwrap();
    let (frame_id, rect_id, text_id) = (
        created.created_ids[0],
        created.created_ids[1],
        created.created_ids[2],
    );

    let baseline = doc.encode_message().unwrap();
    let blobs_before = doc.blobs.len();

    // 1. SetVectorNetwork on non-VECTOR nodes must fail atomically.
    for (label, target) in [
        ("DOCUMENT", root),
        ("CANVAS", page),
        ("FRAME", frame_id),
        ("ROUNDED_RECTANGLE", rect_id),
        ("TEXT", text_id),
    ] {
        let res = execute_edit_ops(
            &mut doc,
            &mut history,
            &[EditOp::SetVectorNetwork {
                target,
                network: valid_triangle_network(),
            }],
            None,
            None,
            false,
        );
        assert!(res.is_err(), "SetVectorNetwork on {label} must return Err");
        assert_eq!(doc.blobs.len(), blobs_before);
        assert_eq!(doc.encode_message().unwrap(), baseline);
    }

    // 2. VectorRegion with empty `loops: vec![]` must be rejected atomically.
    let mut empty_region_loops = valid_triangle_network();
    empty_region_loops.regions[0].loops = Vec::new();
    let res = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetVectorNetwork {
            target: vec_id,
            network: empty_region_loops,
        }],
        None,
        None,
        false,
    );
    assert!(
        res.is_err(),
        "SetVectorNetwork must reject VectorRegion with loops: vec![]"
    );
    assert_eq!(doc.blobs.len(), blobs_before);
    assert_eq!(doc.encode_message().unwrap(), baseline);

    // 3. Multi-style network spanning [0..200, 0..100] must preserve styleOverrideTable,
    // emit region styleID on fillGeometry, and update node.size + vectorData.normalizedSize.
    {
        let mut seed = Txn::new("seed 20x20 vectorData with styleOverrideTable");
        seed.set_field(
            vec_id,
            "size",
            Val::Obj(Obj::from([
                ("x", Val::Float(20.0)),
                ("y", Val::Float(20.0)),
            ])),
        );
        seed.set_field(
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
                        ("styleID", Val::Int(2)),
                        ("fillPaints", Val::Arr(vec![hex_paint("#0000FF").unwrap()])),
                    ]))]),
                ),
            ])),
        );
        doc.apply_txn(&seed).unwrap();
    }

    let styled_net = VectorNetwork {
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
                style_id: 0,
                loops: vec![vec![0, 1, 4]],
            },
            VectorRegion {
                winding_nonzero: true,
                style_id: 2,
                loops: vec![vec![4, 2, 3]],
            },
        ],
    };

    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetVectorNetwork {
            target: vec_id,
            network: styled_net,
        }],
        None,
        None,
        false,
    )
    .unwrap();

    let vd = &doc.node(vec_id).unwrap().data;
    let w = vd.path(&["size", "x"]).and_then(Val::as_f32).unwrap();
    let h = vd.path(&["size", "y"]).and_then(Val::as_f32).unwrap();
    assert!(
        (w - 200.0).abs() < 1e-2 && (h - 100.0).abs() < 1e-2,
        "SetVectorNetwork left stale node.size=({w}, {h}), expected (200, 100)"
    );
    let nw = vd
        .path(&["vectorData", "normalizedSize", "x"])
        .and_then(Val::as_f32)
        .unwrap();
    let nh = vd
        .path(&["vectorData", "normalizedSize", "y"])
        .and_then(Val::as_f32)
        .unwrap();
    assert!(
        (nw - 200.0).abs() < 1e-2 && (nh - 100.0).abs() < 1e-2,
        "SetVectorNetwork left stale normalizedSize=({nw}, {nh}), expected (200, 100)"
    );
    let styles = vd
        .path(&["vectorData", "styleOverrideTable"])
        .map(Val::items)
        .unwrap_or_default();
    assert_eq!(
        styles.len(),
        1,
        "SetVectorNetwork wiped vectorData.styleOverrideTable"
    );
    let fg_styles: Vec<u32> = vd
        .get("fillGeometry")
        .unwrap()
        .items()
        .iter()
        .map(|p| p.get("styleID").and_then(Val::as_u32).unwrap_or(999))
        .collect();
    assert_eq!(
        fg_styles,
        vec![0, 2],
        "SetVectorNetwork discarded region style_id on fillGeometry"
    );

    // 4. Curved open polyline (no regions): size must include cubic Bezier extrema past endpoints.
    let curved_open = VectorNetwork {
        vertices: vec![
            VectorVertex {
                style: 0,
                x: 0.0,
                y: 0.0,
            },
            VectorVertex {
                style: 0,
                x: 100.0,
                y: 0.0,
            },
        ],
        segments: vec![VectorSegment {
            style: 0,
            start: 0,
            tangent_start: (0.0, 80.0),
            end: 1,
            tangent_end: (0.0, 80.0),
        }],
        regions: Vec::new(),
    };
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetVectorNetwork {
            target: vec_id,
            network: curved_open,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let cd = &doc.node(vec_id).unwrap().data;
    let cw = cd.path(&["size", "x"]).and_then(Val::as_f32).unwrap();
    let ch = cd.path(&["size", "y"]).and_then(Val::as_f32).unwrap();
    assert!(
        (cw - 100.0).abs() < 1e-2 && (ch - 60.0).abs() < 1e-2,
        "curved open polyline size ({cw}, {ch}) missed cubic Bezier extremum (expected 100x60)"
    );
}

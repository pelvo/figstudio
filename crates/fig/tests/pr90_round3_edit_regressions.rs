//! Regression tests for PR #90 round-3 edit-core fixes (Batches 2-4).

use std::path::PathBuf;

use figstudio_fig::edit::autolayout::apply_auto_layout_to_doc;
use figstudio_fig::edit::geometry::{encode_path_commands, make_path_entry, rounded_rect_commands};
use figstudio_fig::edit::text_runs::{TextStyleRunPatch, apply_text_range_style};
use figstudio_fig::edit::vector_net::BooleanOp;
use figstudio_fig::edit::{EditOp, GuidAlloc, History, Txn, execute_edit_ops};
use figstudio_fig::tokens::{Tokens, Value};
use figstudio_fig::{Document, Guid, Obj, Val, intern};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn create_rect(doc: &mut Document, history: &mut History, parent: Guid, x: f64, y: f64) -> Guid {
    execute_edit_ops(
        doc,
        history,
        &[EditOp::CreateShape {
            id: None,
            kind: "RECTANGLE".into(),
            parent,
            name: Some("Box".into()),
            x,
            y,
            width: 40.0,
            height: 20.0,
            corner_radius: 0.0,
            fill_hex: Some("#336699".into()),
            count: None,
            star_inner_scale: None,
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0]
}

fn assert_atomic_err(doc: &mut Document, history: &mut History, ops: &[EditOp]) {
    let rev_before = doc.revision;
    let undo_before = history.undo_len();
    let bytes_before = doc.to_bytes().unwrap();
    assert!(
        execute_edit_ops(doc, history, ops, None, None, false).is_err(),
        "expected edit op to fail atomically: {ops:?}"
    );
    assert_eq!(doc.revision, rev_before);
    assert_eq!(history.undo_len(), undo_before);
    assert_eq!(doc.to_bytes().unwrap(), bytes_before);
}

#[test]
fn non_finite_f32_edits_fail_atomically_while_preexisting_nan_survives_delete_undo() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(20);
    let rect = create_rect(&mut doc, &mut history, page, 10.0, 20.0);

    let patch_inf_bits: EditOp = serde_json::from_value(serde_json::json!({
        "op": "patch_raw",
        "target": rect.to_string(),
        "set": { "opacity": { "$float_bits": "7ff0000000000000" } }
    }))
    .unwrap();
    let patch_overflow: EditOp = serde_json::from_value(serde_json::json!({
        "op": "patch_raw",
        "target": rect.to_string(),
        "set": { "opacity": 1e100 }
    }))
    .unwrap();

    for bad in [
        EditOp::Move {
            target: rect,
            x: 1e100,
            y: 0.0,
            relative: false,
        },
        EditOp::Move {
            target: rect,
            x: f64::NAN,
            y: 0.0,
            relative: false,
        },
        EditOp::Resize {
            target: rect,
            width: 1e100,
            height: 10.0,
        },
        EditOp::Resize {
            target: rect,
            width: f64::NAN,
            height: 10.0,
        },
        EditOp::SetLayerProps {
            target: rect,
            visible: None,
            locked: None,
            opacity: Some(f64::INFINITY),
            blend_mode: None,
            clip_content: None,
        },
        EditOp::Rotate {
            target: rect,
            degrees: f64::NAN,
        },
        patch_inf_bits,
        patch_overflow,
    ] {
        assert_atomic_err(&mut doc, &mut history, &[bad]);
    }

    // Seed a pre-existing Figma-authored NaN on the node directly (simulating an opened file).
    let node = doc.nodes.iter_mut().find(|n| n.id == rect).unwrap();
    node.data.set("cornerRadius", Val::Float(f64::NAN));
    let before_delete = doc.to_bytes().unwrap();

    // Deleting and undoing a node with a pre-existing NaN must succeed and round-trip.
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::DeleteNode { target: rect }],
        None,
        None,
        false,
    )
    .unwrap();
    assert!(doc.node(rect).is_none());
    history.undo(&mut doc).unwrap().expect("undo delete");
    assert!(doc.node(rect).is_some());
    assert!(
        doc.node(rect)
            .unwrap()
            .data
            .num("cornerRadius")
            .unwrap()
            .is_nan()
    );
    assert_eq!(doc.to_bytes().unwrap(), before_delete);

    // Overwriting a pre-existing NaN field with a finite value and undoing must also succeed.
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetCornerRadii {
            target: rect,
            radii: [4.0; 4],
            smoothing: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    history.undo(&mut doc).unwrap().expect("undo modify");
    assert!(
        doc.node(rect)
            .unwrap()
            .data
            .num("cornerRadius")
            .unwrap()
            .is_nan()
    );
    history.redo(&mut doc).unwrap().expect("redo modify");
    assert_eq!(doc.node(rect).unwrap().data.num("cornerRadius"), Some(4.0));
}

#[test]
fn set_variable_color_preserves_hex_alpha_across_save_and_reopen() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(20);

    let mode_alpha = Guid(42, 1);
    let mode_opaque = Guid(42, 2);
    let var_id = Guid(42, 3);
    let mut seed = Txn::new("seed color variable");
    seed.create_node(
        var_id,
        Val::Obj(Obj::from([
            ("guid", var_id.to_val()),
            ("phase", Val::Enum(intern("CREATED"))),
            ("type", Val::Enum(intern("VARIABLE"))),
            ("name", Val::Str("Accent".into())),
            ("visible", Val::Bool(true)),
            ("variableResolvedType", Val::Enum(intern("COLOR"))),
            (
                "parentIndex",
                Val::Obj(Obj::from([
                    ("guid", page.to_val()),
                    ("position", Val::Str("~z".into())),
                ])),
            ),
        ])),
    );
    doc.apply_txn(&seed).unwrap();

    execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::SetVariableColor {
                variable: var_id,
                mode: mode_alpha,
                hex: "#33669980".into(),
            },
            EditOp::SetVariableColor {
                variable: var_id,
                mode: mode_opaque,
                hex: "#336699".into(),
            },
        ],
        None,
        None,
        false,
    )
    .unwrap();

    let reopened = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
    let tokens = Tokens::new(&reopened);
    let alias_ref = Val::Obj(Obj::from([(
        "value",
        Val::Obj(Obj::from([("alias", var_id.to_val())])),
    )]));

    let Some(Value::Color(c_alpha)) = tokens.resolve(&alias_ref, Some(mode_alpha)) else {
        panic!("expected resolved color for translucent mode");
    };
    assert!((c_alpha.num("r").unwrap() - 0x33 as f32 / 255.0).abs() < 1e-4);
    assert!((c_alpha.num("g").unwrap() - 0x66 as f32 / 255.0).abs() < 1e-4);
    assert!((c_alpha.num("b").unwrap() - 0x99 as f32 / 255.0).abs() < 1e-4);
    let a = c_alpha.num("a").unwrap();
    assert!(
        (a - 128.0 / 255.0).abs() < 1e-4,
        "expected alpha 128/255, got {a}"
    );

    let Some(Value::Color(c_opaque)) = tokens.resolve(&alias_ref, Some(mode_opaque)) else {
        panic!("expected resolved color for opaque mode");
    };
    assert!((c_opaque.num("a").unwrap() - 1.0).abs() < 1e-5);
}

#[test]
fn rotate_preserves_scale_reflection_and_shear_and_rejects_degenerate_x_basis() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(20);
    let rect = create_rect(&mut doc, &mut history, page, 10.0, 20.0);

    let set_matrix = |doc: &mut Document, id: Guid, m: [f64; 6]| {
        let mut t = Txn::new("set transform");
        t.set_field(
            id,
            "transform",
            Val::Obj(Obj::from([
                ("m00", Val::Float(m[0])),
                ("m01", Val::Float(m[1])),
                ("m02", Val::Float(m[2])),
                ("m10", Val::Float(m[3])),
                ("m11", Val::Float(m[4])),
                ("m12", Val::Float(m[5])),
            ])),
        );
        doc.apply_txn(&t).unwrap();
    };
    let read_matrix = |doc: &Document, id: Guid| -> [f64; 6] {
        let tf = doc.node(id).unwrap().data.get("transform").unwrap();
        [
            f64::from(tf.num("m00").unwrap()),
            f64::from(tf.num("m01").unwrap()),
            f64::from(tf.num("m02").unwrap()),
            f64::from(tf.num("m10").unwrap()),
            f64::from(tf.num("m11").unwrap()),
            f64::from(tf.num("m12").unwrap()),
        ]
    };

    // Case A: scale + Y reflection diag(2, -3) with translation (10, 20).
    for (initial, target_deg) in [
        ([2.0, 0.0, 10.0, 0.0, -3.0, 20.0], 0.0),
        ([2.0, 0.0, 10.0, 0.0, -3.0, 20.0], 90.0),
        ([1.0, 0.5, 10.0, 0.0, 1.5, 20.0], 0.0),
        ([1.0, 0.5, 10.0, 0.0, 1.5, 20.0], 90.0),
    ] {
        set_matrix(&mut doc, rect, initial);
        let det0 = initial[0] * initial[4] - initial[1] * initial[3];
        let col0_len = initial[0].hypot(initial[3]);
        let col1_len = initial[1].hypot(initial[4]);
        let dot0 = initial[0] * initial[1] + initial[3] * initial[4];

        execute_edit_ops(
            &mut doc,
            &mut history,
            &[EditOp::Rotate {
                target: rect,
                degrees: target_deg,
            }],
            None,
            None,
            false,
        )
        .unwrap();

        let got = read_matrix(&doc, rect);
        let det1 = got[0] * got[4] - got[1] * got[3];
        let angle = got[3].atan2(got[0]).to_degrees();
        assert!((got[2] - initial[2]).abs() < 1e-4 && (got[5] - initial[5]).abs() < 1e-4);
        assert!(
            (det1 - det0).abs() < 1e-4,
            "det changed: {det0} -> {det1} for initial={initial:?} deg={target_deg}"
        );
        assert!((got[0].hypot(got[3]) - col0_len).abs() < 1e-4);
        assert!((got[1].hypot(got[4]) - col1_len).abs() < 1e-4);
        assert!(((got[0] * got[1] + got[3] * got[4]) - dot0).abs() < 1e-4);
        assert!(
            (angle - target_deg).abs() < 1e-3,
            "expected angle {target_deg}, got {angle}"
        );
    }

    // Case B: degenerate zero X-basis must fail atomically.
    set_matrix(&mut doc, rect, [0.0, 0.0, 10.0, 0.0, 1.0, 20.0]);
    assert_atomic_err(
        &mut doc,
        &mut history,
        &[EditOp::Rotate {
            target: rect,
            degrees: 45.0,
        }],
    );
}

fn create_sized_shape(
    doc: &mut Document,
    history: &mut History,
    kind: &str,
    parent: Guid,
    [x, y, w, h]: [f64; 4],
) -> Guid {
    execute_edit_ops(
        doc,
        history,
        &[EditOp::CreateShape {
            id: None,
            kind: kind.into(),
            parent,
            name: None,
            x,
            y,
            width: w,
            height: h,
            corner_radius: 0.0,
            fill_hex: Some("#336699".into()),
            count: None,
            star_inner_scale: None,
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0]
}

fn node_size(doc: &Document, id: Guid) -> (f64, f64) {
    let d = &doc.node(id).unwrap().data;
    (
        f64::from(d.path(&["size", "x"]).and_then(Val::as_f32).unwrap()),
        f64::from(d.path(&["size", "y"]).and_then(Val::as_f32).unwrap()),
    )
}

#[test]
fn boolean_combine_matches_recompute_on_hidden_operands_and_rejects_nonempty_frame() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(30);

    // 1. Union(visible_a, hidden_b) matches recompute after moving visible_a.
    let vis_a = create_sized_shape(
        &mut doc,
        &mut history,
        "RECTANGLE",
        page,
        [0.0, 0.0, 20.0, 20.0],
    );
    let hid_b = create_sized_shape(
        &mut doc,
        &mut history,
        "RECTANGLE",
        page,
        [100.0, 0.0, 20.0, 20.0],
    );
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetLayerProps {
            target: hid_b,
            visible: Some(false),
            locked: None,
            opacity: None,
            blend_mode: None,
            clip_content: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let bool_u = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Union,
            a: vis_a,
            b: hid_b,
            name: None,
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let initial_u = node_size(&doc, bool_u);
    assert!(
        (initial_u.0 - 20.0).abs() < 1e-3,
        "Union with hidden operand included hidden bounds: {initial_u:?}"
    );
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::Move {
            target: vis_a,
            x: 1.0,
            y: 0.0,
            relative: true,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    assert!((node_size(&doc, bool_u).0 - 20.0).abs() < 1e-3);

    // 2. Subtract(hidden_a, visible_b) matches recompute after moving visible_b.
    let hid_a = create_sized_shape(
        &mut doc,
        &mut history,
        "RECTANGLE",
        page,
        [0.0, 50.0, 20.0, 20.0],
    );
    let vis_b = create_sized_shape(
        &mut doc,
        &mut history,
        "RECTANGLE",
        page,
        [50.0, 50.0, 20.0, 20.0],
    );
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetLayerProps {
            target: hid_a,
            visible: Some(false),
            locked: None,
            opacity: None,
            blend_mode: None,
            clip_content: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let bool_s = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Subtract,
            a: hid_a,
            b: vis_b,
            name: None,
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let before_s = node_size(&doc, bool_s);
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::Move {
            target: vis_b,
            x: 1.0,
            y: 0.0,
            relative: true,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let after_s = node_size(&doc, bool_s);
    assert!(
        (before_s.0 - after_s.0).abs() < 1e-3 && (before_s.0 - 20.0).abs() < 1e-3,
        "Subtract(hidden, visible) diverged between creation {before_s:?} and recompute {after_s:?}"
    );

    // 3. Non-empty FRAME is rejected atomically at BooleanCombine without reparenting.
    let frame = create_sized_shape(
        &mut doc,
        &mut history,
        "FRAME",
        page,
        [0.0, 100.0, 40.0, 40.0],
    );
    let _child = create_sized_shape(
        &mut doc,
        &mut history,
        "RECTANGLE",
        frame,
        [5.0, 5.0, 10.0, 10.0],
    );
    let sibling = create_sized_shape(
        &mut doc,
        &mut history,
        "RECTANGLE",
        page,
        [20.0, 100.0, 40.0, 40.0],
    );
    assert_atomic_err(
        &mut doc,
        &mut history,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Union,
            a: frame,
            b: sibling,
            name: None,
        }],
    );
    assert_eq!(doc.node(frame).unwrap().parent, Some(page));
    assert_eq!(doc.node(sibling).unwrap().parent, Some(page));
}

#[test]
fn text_edits_inside_visible_boolean_fail_closed_and_outside_drop_stale_fill_geometry() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(30);

    let seed_text_fill = |doc: &mut Document, id: Guid, w: f32, h: f32| {
        let mut t = Txn::new("seed baked text fillGeometry");
        let blob = doc.blobs.len();
        t.add_blobs
            .push(encode_path_commands(&rounded_rect_commands(
                w, h, [0.0; 4], 0.0,
            )));
        t.set_field(
            id,
            "fillGeometry",
            Val::Arr(vec![make_path_entry(blob, false)]),
        );
        doc.apply_txn(&t).unwrap();
    };

    // Standalone TEXT outside boolean drops stale fillGeometry when text changes.
    let standalone = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateText {
            id: None,
            parent: page,
            x: 0.0,
            y: 0.0,
            characters: "Hi".into(),
            font_family: "Inter".into(),
            font_style: "Regular".into(),
            font_size: 16.0,
            fill_hex: "#000000".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    seed_text_fill(&mut doc, standalone, 20.0, 20.0);
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetText {
            target: standalone,
            characters: "Changed".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    assert!(
        doc.node(standalone)
            .unwrap()
            .data
            .get("fillGeometry")
            .is_none(),
        "stale TEXT fillGeometry survived SetText outside boolean"
    );

    // Visible TEXT inside BOOLEAN_OPERATION (both NONE and WIDTH_AND_HEIGHT): outline edits fail atomically.
    for auto_resize in ["NONE", "WIDTH_AND_HEIGHT"] {
        let txt = execute_edit_ops(
            &mut doc,
            &mut history,
            &[EditOp::CreateText {
                id: None,
                parent: page,
                x: 0.0,
                y: 0.0,
                characters: "I".into(),
                font_family: "Inter".into(),
                font_style: "Regular".into(),
                font_size: 16.0,
                fill_hex: "#000000".into(),
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
            &[EditOp::SetTypography {
                target: txt,
                font_family: None,
                font_style: None,
                font_size: None,
                align_horizontal: None,
                auto_resize: Some(auto_resize.into()),
            }],
            None,
            None,
            false,
        )
        .unwrap();
        seed_text_fill(&mut doc, txt, 10.0, 20.0);
        let rect = create_sized_shape(
            &mut doc,
            &mut history,
            "RECTANGLE",
            page,
            [30.0, 0.0, 20.0, 20.0],
        );
        let _bool_id = execute_edit_ops(
            &mut doc,
            &mut history,
            &[EditOp::BooleanCombine {
                operation: BooleanOp::Union,
                a: txt,
                b: rect,
                name: None,
            }],
            None,
            None,
            false,
        )
        .unwrap()
        .0
        .created_ids[0];

        let raw_text_data: EditOp = serde_json::from_value(serde_json::json!({
            "op": "patch_raw",
            "target": txt.to_string(),
            "set": { "textData": { "characters": "RAW" } }
        }))
        .unwrap();

        for bad in [
            EditOp::SetText {
                target: txt,
                characters: "OOOOOO".into(),
            },
            EditOp::SpliceText {
                target: txt,
                utf16_start: 0,
                utf16_delete_len: 1,
                insert: "WWWW".into(),
            },
            EditOp::StyleTextRange {
                target: txt,
                utf16_start: 0,
                utf16_end: 1,
                style: TextStyleRunPatch {
                    font_size: Some(48.0),
                    ..Default::default()
                },
            },
            EditOp::SetTypography {
                target: txt,
                font_family: None,
                font_style: None,
                font_size: Some(48.0),
                align_horizontal: None,
                auto_resize: Some("WIDTH_AND_HEIGHT".into()),
            },
            raw_text_data,
        ] {
            assert_atomic_err(&mut doc, &mut history, &[bad]);
        }

        // Metadata / fill-paint edits on the visible TEXT operand still succeed.
        execute_edit_ops(
            &mut doc,
            &mut history,
            &[
                EditOp::Rename {
                    target: txt,
                    name: "Label".into(),
                },
                EditOp::SetSolidFill {
                    target: txt,
                    hex: Some("#FF0000".into()),
                },
            ],
            None,
            None,
            false,
        )
        .unwrap();

        // Hiding the TEXT operand allows subsequent text edits.
        execute_edit_ops(
            &mut doc,
            &mut history,
            &[
                EditOp::SetLayerProps {
                    target: txt,
                    visible: Some(false),
                    locked: None,
                    opacity: None,
                    blend_mode: None,
                    clip_content: None,
                },
                EditOp::SetText {
                    target: txt,
                    characters: "HiddenEdit".into(),
                },
            ],
            None,
            None,
            false,
        )
        .unwrap();
    }
}

#[test]
fn parent_resize_and_autolayout_reject_boolean_size_changes_atomically() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(30);

    // 1. Parent Resize with SCALE/STRETCH boolean child rejects atomically on constrained axis.
    let frame = create_sized_shape(
        &mut doc,
        &mut history,
        "FRAME",
        page,
        [0.0, 0.0, 100.0, 100.0],
    );
    let a = create_sized_shape(
        &mut doc,
        &mut history,
        "RECTANGLE",
        frame,
        [10.0, 10.0, 20.0, 20.0],
    );
    let b = create_sized_shape(
        &mut doc,
        &mut history,
        "RECTANGLE",
        frame,
        [20.0, 10.0, 20.0, 20.0],
    );
    let bool_child = execute_edit_ops(
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
    )
    .unwrap()
    .0
    .created_ids[0];

    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetConstraints {
            target: bool_child,
            horizontal: Some("SCALE".into()),
            vertical: Some("MIN".into()),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    // Resizing along constrained horizontal axis fails atomically.
    assert_atomic_err(
        &mut doc,
        &mut history,
        &[EditOp::Resize {
            target: frame,
            width: 200.0,
            height: 100.0,
        }],
    );
    // Resizing only the unconstrained vertical axis succeeds.
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::Resize {
            target: frame,
            width: 100.0,
            height: 160.0,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    assert_eq!(node_size(&doc, frame), (100.0, 160.0));

    // MIN/CENTER/MAX constraints keep boolean size unchanged and succeed on both axes.
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetConstraints {
            target: bool_child,
            horizontal: Some("MAX".into()),
            vertical: Some("CENTER".into()),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::Resize {
            target: frame,
            width: 150.0,
            height: 200.0,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    assert_eq!(node_size(&doc, frame), (150.0, 200.0));

    // 2. Auto-Layout container with imported FILL/STRETCH boolean child rejects when size changes,
    //    succeeds when computed size equals current size.
    let stack = create_sized_shape(
        &mut doc,
        &mut history,
        "FRAME",
        page,
        [0.0, 300.0, 200.0, 20.0],
    );
    let ba = create_sized_shape(
        &mut doc,
        &mut history,
        "RECTANGLE",
        stack,
        [0.0, 0.0, 20.0, 20.0],
    );
    let bb = create_sized_shape(
        &mut doc,
        &mut history,
        "RECTANGLE",
        stack,
        [10.0, 0.0, 20.0, 20.0],
    );
    let al_bool = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::BooleanCombine {
            operation: BooleanOp::Union,
            a: ba,
            b: bb,
            name: None,
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let sib = create_sized_shape(
        &mut doc,
        &mut history,
        "RECTANGLE",
        stack,
        [50.0, 0.0, 170.0, 20.0],
    );
    let mut seed = Txn::new("imported FILL/STRETCH boolean in FIXED stack");
    seed.set_field(stack, "stackMode", Val::Enum(intern("HORIZONTAL")));
    seed.set_field(stack, "stackPrimarySizing", Val::Enum(intern("FIXED")));
    seed.set_field(stack, "stackCounterSizing", Val::Enum(intern("FIXED")));
    seed.set_field(al_bool, "stackChildPrimaryGrow", Val::Float(1.0));
    seed.set_field(al_bool, "stackChildAlignSelf", Val::Enum(intern("STRETCH")));
    doc.apply_txn(&seed).unwrap();

    // Currently stack=200x20, sib=170x20, al_bool=30x20 -> computed fill size is 30x20 (unchanged!).
    apply_auto_layout_to_doc(&mut doc, Some(stack)).unwrap();
    assert_eq!(node_size(&doc, al_bool), (30.0, 20.0));

    // Resizing sibling to 100x20 would force al_bool to grow to 100x20 -> must fail atomically.
    assert_atomic_err(
        &mut doc,
        &mut history,
        &[EditOp::Resize {
            target: sib,
            width: 100.0,
            height: 20.0,
        }],
    );
}

#[test]
fn constrained_primary_fill_is_independent_of_sibling_order() {
    let run_case = |specs: &[(&str, Option<f64>, Option<f64>)]| -> Vec<(String, f64)> {
        let mut doc = Document::open(fixture("structure.fig")).unwrap();
        let page = doc.pages[0];
        let mut history = History::new(20);
        let stack = create_sized_shape(
            &mut doc,
            &mut history,
            "FRAME",
            page,
            [0.0, 0.0, 100.0, 40.0],
        );
        execute_edit_ops(
            &mut doc,
            &mut history,
            &[EditOp::SetAutoLayout {
                target: stack,
                mode: "HORIZONTAL".into(),
                spacing: Some(0.0),
                counter_spacing: None,
                padding: Some([0.0, 0.0, 0.0, 0.0]),
                primary_sizing: Some("FIXED".into()),
                counter_sizing: Some("FIXED".into()),
                primary_align: None,
                counter_align: None,
                wrap: Some(false),
            }],
            None,
            None,
            false,
        )
        .unwrap();
        let mut ids = Vec::new();
        for &(label, min_w, max_w) in specs {
            let cid = create_sized_shape(
                &mut doc,
                &mut history,
                "RECTANGLE",
                stack,
                [0.0, 0.0, 10.0, 20.0],
            );
            execute_edit_ops(
                &mut doc,
                &mut history,
                &[EditOp::SetAutoLayoutChild {
                    target: cid,
                    primary_fill: Some(true),
                    counter_stretch: None,
                    positioning: None,
                    min_size: min_w.map(|m| [m, 0.0]),
                    max_size: max_w.map(|m| [m, 1000.0]),
                }],
                None,
                None,
                false,
            )
            .unwrap();
            ids.push((label.to_string(), cid));
        }
        apply_auto_layout_to_doc(&mut doc, Some(stack)).unwrap();
        let mut out: Vec<(String, f64)> = ids
            .into_iter()
            .map(|(lbl, id)| (lbl, node_size(&doc, id).0))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    };

    // 2-child case: [A(max=10), B(min=50)] vs [B, A] -> A=10, B=90 in both orders.
    let a = ("A", None, Some(10.0));
    let b = ("B", Some(50.0), None);
    for order in [[a, b], [b, a]] {
        let got = run_case(&order);
        assert!(
            (got[0].1 - 10.0).abs() < 1e-3 && (got[1].1 - 90.0).abs() < 1e-3,
            "order {order:?} produced {got:?}, expected A=10, B=90"
        );
    }

    // 3-child case: all 6 permutations of A(max=10), B(min=35), C(free) -> A=10, B=45, C=45.
    let a3 = ("A", None, Some(10.0));
    let b3 = ("B", Some(35.0), None);
    let c3 = ("C", None, None);
    for order in [
        [a3, b3, c3],
        [a3, c3, b3],
        [b3, a3, c3],
        [b3, c3, a3],
        [c3, a3, b3],
        [c3, b3, a3],
    ] {
        let got = run_case(&order);
        assert!(
            (got[0].1 - 10.0).abs() < 1e-3
                && (got[1].1 - 45.0).abs() < 1e-3
                && (got[2].1 - 45.0).abs() < 1e-3,
            "permutation {order:?} produced {got:?}, expected A=10, B=45, C=45"
        );
    }
}

#[test]
fn set_typography_resizes_auto_sized_text_in_hug_stack_and_keeps_none_fixed() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(30);

    let stack = create_sized_shape(
        &mut doc,
        &mut history,
        "FRAME",
        page,
        [0.0, 0.0, 200.0, 40.0],
    );
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetAutoLayout {
            target: stack,
            mode: "HORIZONTAL".into(),
            spacing: Some(10.0),
            counter_spacing: None,
            padding: Some([0.0, 0.0, 0.0, 0.0]),
            primary_sizing: Some("HUG".into()),
            counter_sizing: Some("HUG".into()),
            primary_align: None,
            counter_align: None,
            wrap: Some(false),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let txt = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateText {
            id: None,
            parent: stack,
            x: 0.0,
            y: 0.0,
            characters: "Hello World".into(),
            font_family: "Inter".into(),
            font_style: "Regular".into(),
            font_size: 12.0,
            fill_hex: "#111111".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    let sib = create_sized_shape(
        &mut doc,
        &mut history,
        "RECTANGLE",
        stack,
        [0.0, 0.0, 20.0, 20.0],
    );

    let txt_sz_before = node_size(&doc, txt);
    let stack_sz_before = node_size(&doc, stack);
    let sib_x_before = f64::from(
        doc.node(sib)
            .unwrap()
            .data
            .path(&["transform", "m02"])
            .and_then(Val::as_f32)
            .unwrap(),
    );

    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetTypography {
            target: txt,
            font_family: None,
            font_style: None,
            font_size: Some(24.0),
            align_horizontal: None,
            auto_resize: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();

    let txt_sz_after = node_size(&doc, txt);
    let stack_sz_after = node_size(&doc, stack);
    let sib_x_after = f64::from(
        doc.node(sib)
            .unwrap()
            .data
            .path(&["transform", "m02"])
            .and_then(Val::as_f32)
            .unwrap(),
    );
    assert!(
        txt_sz_after.0 > txt_sz_before.0 && txt_sz_after.1 > txt_sz_before.1,
        "WIDTH_AND_HEIGHT text box did not grow: {txt_sz_before:?} -> {txt_sz_after:?}"
    );
    assert!(
        sib_x_after > sib_x_before,
        "sibling did not shift right: {sib_x_before} -> {sib_x_after}"
    );
    assert!(
        stack_sz_after.0 > stack_sz_before.0,
        "HUG stack width did not grow: {stack_sz_before:?} -> {stack_sz_after:?}"
    );

    // Switch to NONE, set a fixed box size, and verify SetTypography(font_size) keeps it fixed.
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::SetTypography {
                target: txt,
                font_family: None,
                font_style: None,
                font_size: None,
                align_horizontal: None,
                auto_resize: Some("NONE".into()),
            },
            EditOp::Resize {
                target: txt,
                width: 90.0,
                height: 30.0,
            },
        ],
        None,
        None,
        false,
    )
    .unwrap();
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetTypography {
            target: txt,
            font_family: None,
            font_style: None,
            font_size: Some(36.0),
            align_horizontal: None,
            auto_resize: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    assert_eq!(node_size(&doc, txt), (90.0, 30.0));

    // Switching NONE -> WIDTH_AND_HEIGHT recomputes the auto-resized dimensions.
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetTypography {
            target: txt,
            font_family: None,
            font_style: None,
            font_size: None,
            align_horizontal: None,
            auto_resize: Some("WIDTH_AND_HEIGHT".into()),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    assert!(node_size(&doc, txt).0 > 90.0);
}

#[test]
fn style_text_range_rejects_surrogate_pair_splits_and_accepts_full_scalar_span() {
    let patch = TextStyleRunPatch {
        font_size: Some(20.0),
        ..Default::default()
    };
    let td = Val::Obj(Obj::from([("characters", Val::Str("🦀".into()))]));

    for (start, end) in [(1, 2), (0, 1), (1, 1)] {
        assert!(
            apply_text_range_style(&td, start, end, &patch).is_err(),
            "expected range [{start}, {end}) on surrogate pair to fail"
        );
    }
    let ok = apply_text_range_style(&td, 0, 2, &patch).expect("full surrogate pair range");
    let ids: Vec<u32> = ok
        .get("characterStyleIDs")
        .unwrap()
        .items()
        .iter()
        .map(|v| v.as_u32().unwrap())
        .collect();
    assert_eq!(ids, vec![1, 1]);

    // Also verify via public execute_edit_ops boundary.
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(10);
    let txt = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateText {
            id: None,
            parent: page,
            x: 0.0,
            y: 0.0,
            characters: "🦀".into(),
            font_family: "Inter".into(),
            font_style: "Regular".into(),
            font_size: 16.0,
            fill_hex: "#000000".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    for (start, end) in [(1, 2), (0, 1), (1, 1)] {
        assert_atomic_err(
            &mut doc,
            &mut history,
            &[EditOp::StyleTextRange {
                target: txt,
                utf16_start: start,
                utf16_end: end,
                style: patch.clone(),
            }],
        );
    }
}

#[test]
fn guid_allocator_never_collides_at_u32_max_session_boundary() {
    for high_locals in [
        [Guid(u32::MAX, 1), Guid(u32::MAX, u32::MAX - 1)],
        [Guid(u32::MAX, 1), Guid(u32::MAX, u32::MAX)],
    ] {
        let mut doc = Document::open(fixture("structure.fig")).unwrap();
        let page = doc.pages[0];
        let mut history = History::new(10);
        for (i, gid) in high_locals.into_iter().enumerate() {
            let mut t = Txn::new("seed high guid");
            t.create_node(
                gid,
                Val::Obj(Obj::from([
                    ("guid", gid.to_val()),
                    ("phase", Val::Enum(intern("CREATED"))),
                    ("type", Val::Enum(intern("ROUNDED_RECTANGLE"))),
                    ("name", Val::Str(format!("High {i}"))),
                    ("visible", Val::Bool(true)),
                    (
                        "parentIndex",
                        Val::Obj(Obj::from([
                            ("guid", page.to_val()),
                            ("position", Val::Str(format!("~{i}"))),
                        ])),
                    ),
                ])),
            );
            doc.apply_txn(&t).unwrap();
        }
        let next = GuidAlloc::for_document(&doc).next_guid();
        assert!(
            doc.node(next).is_none(),
            "GuidAlloc returned already-occupied guid {next} for seeded {high_locals:?}"
        );
        // Real CreateShape op succeeds without GUID collision.
        let created = create_rect(&mut doc, &mut history, page, 0.0, 0.0);
        assert!(doc.node(created).is_some());
    }
}

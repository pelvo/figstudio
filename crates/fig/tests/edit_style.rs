//! Paint lists, stroke style and effect lists.

use std::path::PathBuf;

use figstudio_fig::edit::{
    EditOp, EffectPatch, EffectSpec, GradientKind, History, ImageScale, PaintPatch, PaintSlot,
    PaintSpec, StopSpec, Txn, execute_edit_ops,
};
use figstudio_fig::{Document, Guid, Val};

fn open(name: &str) -> Document {
    Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(format!("{name}.fig")),
    )
    .unwrap()
}

fn run(doc: &mut Document, h: &mut History, ops: Vec<EditOp>) -> Result<Vec<Guid>, String> {
    execute_edit_ops(doc, h, &ops, None, None, false)
        .map(|r| r.0.created_ids)
        .map_err(|e| e.to_string())
}

/// `structure.fig` and a new rectangle on its first page.
fn rect() -> (Document, History, Guid) {
    let mut doc = open("structure");
    let mut h = History::new(50);
    let page = doc.pages[0];
    let id = run(
        &mut doc,
        &mut h,
        vec![EditOp::CreateShape {
            id: None,
            kind: "RECTANGLE".into(),
            parent: page,
            name: None,
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }],
    )
    .unwrap()[0];
    (doc, h, id)
}

fn list<'a>(doc: &'a Document, id: Guid, field: &str) -> &'a [Val] {
    doc.node(id)
        .unwrap()
        .data
        .get(field)
        .map(Val::items)
        .unwrap_or_default()
}

fn solid(hex: &str) -> PaintSpec {
    PaintSpec::Solid {
        hex: hex.into(),
        opacity: None,
        visible: None,
        blend_mode: None,
    }
}

fn first_image_hash(doc: &Document) -> String {
    doc.nodes
        .iter()
        .find_map(|n| {
            n.data
                .get("fillPaints")?
                .items()
                .iter()
                .find_map(|p| Some(hex(p.path(&["image", "hash"])?.as_bytes()?)))
        })
        .unwrap()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn set_paints_writes_solid_gradient_and_image_paints_that_survive_a_save() {
    let (mut doc, mut h, id) = rect();
    let hash = first_image_hash(&doc);
    run(
        &mut doc,
        &mut h,
        vec![EditOp::SetPaints {
            target: id,
            slot: PaintSlot::Fill,
            paints: vec![
                solid("#FF000080"),
                PaintSpec::Gradient {
                    kind: GradientKind::Radial,
                    stops: vec![
                        StopSpec {
                            position: 0.0,
                            hex: "#FFFFFF".into(),
                        },
                        StopSpec {
                            position: 1.0,
                            hex: "#00000000".into(),
                        },
                    ],
                    from: None,
                    to: None,
                    opacity: Some(0.5),
                    visible: None,
                    blend_mode: Some("MULTIPLY".into()),
                },
                PaintSpec::Image {
                    hash: hash.clone(),
                    scale: ImageScale::Crop,
                    rotation: Some(90.0),
                    tile_scale: None,
                    filters: Some([("saturation".to_string(), -0.5)].into()),
                    opacity: None,
                    visible: Some(false),
                    blend_mode: None,
                },
            ],
        }],
    )
    .unwrap();
    let reopened = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
    let paints = list(&reopened, id, "fillPaints");
    assert_eq!(paints.len(), 3);
    assert_eq!(paints[0].text("type"), Some("SOLID"));
    assert!((paints[0].num("opacity").unwrap() - 128.0 / 255.0).abs() < 1e-3);
    assert_eq!(paints[1].text("type"), Some("GRADIENT_RADIAL"));
    assert_eq!(paints[1].text("blendMode"), Some("MULTIPLY"));
    assert_eq!(paints[1].get("stops").unwrap().items().len(), 2);
    assert_eq!(paints[2].text("imageScaleMode"), Some("STRETCH"));
    assert_eq!(paints[2].flag("visible"), Some(false));
    assert!(paints[2].path(&["paintFilter", "vibrance"]).is_some());
    assert_eq!(
        hex(paints[2]
            .path(&["image", "hash"])
            .unwrap()
            .as_bytes()
            .unwrap()),
        hash
    );
    h.undo(&mut doc).unwrap();
    assert_eq!(list(&doc, id, "fillPaints").len(), 1);
}

#[test]
fn paint_list_edits_keep_other_fields_and_check_indices() {
    let (mut doc, mut h, id) = rect();
    // An extra field on the existing paint must survive an update.
    let mut paints: Vec<Val> = list(&doc, id, "fillPaints").to_vec();
    paints[0].set("imageShouldColorManage", Val::Bool(true));
    let mut seed = Txn::new("seed");
    seed.set_field(id, "fillPaints", Val::Arr(paints));
    doc.apply_txn(&seed).unwrap();

    run(
        &mut doc,
        &mut h,
        vec![
            EditOp::AddPaint {
                target: id,
                slot: PaintSlot::Fill,
                paint: solid("#00FF00"),
                index: Some(0),
            },
            EditOp::UpdatePaint {
                target: id,
                slot: PaintSlot::Fill,
                index: 1,
                patch: PaintPatch {
                    opacity: Some(0.25),
                    hex: Some("#123456".into()),
                    ..Default::default()
                },
            },
            EditOp::MovePaint {
                target: id,
                slot: PaintSlot::Fill,
                from: 1,
                to: 0,
            },
        ],
    )
    .unwrap();
    let paints = list(&doc, id, "fillPaints");
    assert_eq!(paints.len(), 2);
    assert_eq!(paints[0].flag("imageShouldColorManage"), Some(true));
    assert!((paints[0].num("opacity").unwrap() - 0.25).abs() < 1e-6);
    assert!(
        (paints[0].path(&["color", "b"]).unwrap().as_f32().unwrap() - 0x56 as f32 / 255.0).abs()
            < 1e-3
    );

    let bad = |op| run(&mut doc.clone(), &mut History::new(5), vec![op]).is_err();
    assert!(bad(EditOp::RemovePaint {
        target: id,
        slot: PaintSlot::Fill,
        index: 2
    }));
    assert!(bad(EditOp::MovePaint {
        target: id,
        slot: PaintSlot::Stroke,
        from: 0,
        to: 0
    }));
    assert!(bad(EditOp::UpdatePaint {
        target: id,
        slot: PaintSlot::Fill,
        index: 0,
        patch: PaintPatch {
            stops: Some(vec![]),
            ..Default::default()
        }
    }));
    assert!(bad(EditOp::AddPaint {
        target: id,
        slot: PaintSlot::Fill,
        paint: solid("not a colour"),
        index: None
    }));
    assert!(bad(EditOp::UpdatePaint {
        target: id,
        slot: PaintSlot::Fill,
        index: 0,
        patch: PaintPatch {
            blend_mode: Some("NOT_A_MODE".into()),
            ..Default::default()
        }
    }));
}

#[test]
fn editing_a_styled_list_detaches_its_style() {
    let mut doc = open("motion");
    let mut h = History::new(5);
    let fill = doc
        .nodes
        .iter()
        .find(|n| n.data.get("styleIdForFill").is_some())
        .unwrap()
        .id;
    let effect = doc
        .nodes
        .iter()
        .find(|n| n.data.get("styleIdForEffect").is_some() && n.data.get("effects").is_some())
        .unwrap()
        .id;
    run(
        &mut doc,
        &mut h,
        vec![
            EditOp::AddPaint {
                target: fill,
                slot: PaintSlot::Fill,
                paint: solid("#FFFFFF"),
                index: None,
            },
            EditOp::UpdateEffect {
                target: effect,
                index: 0,
                patch: EffectPatch {
                    visible: Some(false),
                    ..Default::default()
                },
            },
        ],
    )
    .unwrap();
    assert!(doc.node(fill).unwrap().data.get("styleIdForFill").is_none());
    assert!(
        doc.node(effect)
            .unwrap()
            .data
            .get("styleIdForEffect")
            .is_none()
    );
}

#[test]
fn stroke_style_sets_caps_dashes_and_sides_and_drops_stale_geometry() {
    let mut doc = open("vector");
    let mut h = History::new(5);
    let v: Guid = "0:5".parse().unwrap();
    assert!(doc.node(v).unwrap().data.get("strokeGeometry").is_some());
    run(
        &mut doc,
        &mut h,
        vec![EditOp::SetStrokeStyle {
            target: v,
            weight: Some(3.0),
            align: Some("center".into()),
            cap: Some("round".into()),
            join: Some("bevel".into()),
            miter_limit: Some(4.0),
            dashes: Some(vec![4.0, 2.0]),
            sides: None,
        }],
    )
    .unwrap();
    let d = &doc.node(v).unwrap().data;
    assert!(d.get("strokeGeometry").is_none());
    assert_eq!(d.text("strokeCap"), Some("ROUND"));
    assert_eq!(d.text("strokeJoin"), Some("BEVEL"));
    assert_eq!(d.get("dashPattern").unwrap().items().len(), 2);

    let (mut doc, mut h, id) = rect();
    run(
        &mut doc,
        &mut h,
        vec![EditOp::SetStrokeStyle {
            target: id,
            weight: None,
            align: None,
            cap: None,
            join: None,
            miter_limit: None,
            dashes: Some(vec![]),
            sides: Some([1.0, 2.0, 3.0, 4.0]),
        }],
    )
    .unwrap();
    let d = &doc.node(id).unwrap().data;
    assert_eq!(d.flag("borderStrokeWeightsIndependent"), Some(true));
    assert_eq!(d.num("borderLeftWeight"), Some(4.0));
    assert!(d.get("dashPattern").is_none());
    assert!(
        run(
            &mut doc,
            &mut h,
            vec![EditOp::SetStrokeStyle {
                target: id,
                weight: Some(-1.0),
                align: None,
                cap: None,
                join: None,
                miter_limit: None,
                dashes: None,
                sides: None,
            }]
        )
        .is_err()
    );
}

#[test]
fn effect_lists_can_be_set_added_updated_moved_and_removed() {
    let (mut doc, mut h, id) = rect();
    run(
        &mut doc,
        &mut h,
        vec![EditOp::SetEffects {
            target: id,
            drop_shadow: None,
            blur_radius: None,
            background_blur: false,
            effects: Some(vec![
                EffectSpec::DropShadow {
                    hex: Some("#FF000080".into()),
                    offset: Some([2.0, 3.0]),
                    radius: Some(6.0),
                    spread: None,
                    show_behind: None,
                    visible: None,
                    blend_mode: None,
                },
                EffectSpec::LayerBlur {
                    radius: 2.0,
                    visible: None,
                },
            ]),
        }],
    )
    .unwrap();
    run(
        &mut doc,
        &mut h,
        vec![
            EditOp::AddEffect {
                target: id,
                effect: EffectSpec::InnerShadow {
                    hex: None,
                    offset: None,
                    radius: None,
                    spread: Some(1.0),
                    visible: None,
                    blend_mode: None,
                },
                index: Some(1),
            },
            EditOp::UpdateEffect {
                target: id,
                index: 0,
                patch: EffectPatch {
                    radius: Some(10.0),
                    ..Default::default()
                },
            },
            EditOp::MoveEffect {
                target: id,
                from: 2,
                to: 0,
            },
            EditOp::RemoveEffect {
                target: id,
                index: 2,
            },
        ],
    )
    .unwrap();
    let reopened = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
    let fx = list(&reopened, id, "effects");
    let kinds: Vec<_> = fx.iter().map(|e| e.text("type").unwrap()).collect();
    assert_eq!(kinds, ["FOREGROUND_BLUR", "DROP_SHADOW"]);
    assert_eq!(fx[1].num("radius"), Some(10.0));
    assert_eq!(
        fx[1].path(&["offset", "y"]).and_then(Val::as_f32),
        Some(3.0)
    );
    // The patch refuses fields a blur does not have; the shorthand form still works.
    assert!(
        run(
            &mut doc,
            &mut h,
            vec![EditOp::UpdateEffect {
                target: id,
                index: 0,
                patch: EffectPatch {
                    hex: Some("#000000".into()),
                    ..Default::default()
                },
            }]
        )
        .is_err()
    );
    let op: EditOp = serde_json::from_value(serde_json::json!({
        "op": "set_effects", "target": id.to_string(), "blur_radius": 4.0
    }))
    .unwrap();
    run(&mut doc, &mut h, vec![op]).unwrap();
    assert_eq!(list(&doc, id, "effects").len(), 1);
}

#[test]
fn paint_and_effect_specs_have_a_readable_json_form() {
    let op: EditOp = serde_json::from_value(serde_json::json!({
        "op": "add_paint", "target": "1:2", "slot": "fill",
        "paint": { "type": "gradient", "kind": "linear",
                   "stops": [{ "position": 0, "hex": "#000" }, { "position": 1, "hex": "#FFFFFF" }],
                   "from": [0, 0], "to": [1, 1] }
    }))
    .unwrap();
    assert!(matches!(op, EditOp::AddPaint { .. }));
    let op: EditOp = serde_json::from_value(serde_json::json!({
        "op": "add_effect", "target": "1:2",
        "effect": { "type": "drop_shadow", "hex": "#00000040", "offset": [0, 4], "radius": 8 }
    }))
    .unwrap();
    assert!(matches!(op, EditOp::AddEffect { .. }));
}

#[test]
fn whole_node_typography_replaces_the_same_setting_on_character_runs() {
    let mut doc = open("structure");
    let mut h = History::new(10);
    let text = doc.nodes.iter().find(|n| n.kind == "TEXT").unwrap().id;
    // Style the first character so the text has a run override.
    run(
        &mut doc,
        &mut h,
        vec![EditOp::StyleTextRange {
            target: text,
            utf16_start: 0,
            utf16_end: 1,
            style: figstudio_fig::edit::text_runs::TextStyleRunPatch {
                letter_spacing_px: Some(3.0),
                font_size: Some(30.0),
                ..Default::default()
            },
        }],
    )
    .unwrap();
    let op: EditOp = serde_json::from_value(serde_json::json!({
        "op": "set_typography", "target": text.to_string(),
        "line_height": "auto", "letter_spacing": { "percent": 2.0 },
        "paragraph_spacing": 8, "align_vertical": "center", "decoration": "underline",
        "case": "upper", "truncate": true, "max_lines": 2
    }))
    .unwrap();
    run(&mut doc, &mut h, vec![op]).unwrap();
    let d = &doc.node(text).unwrap().data;
    assert_eq!(
        d.path(&["lineHeight", "units"]).and_then(Val::as_str),
        Some("PERCENT")
    );
    assert_eq!(
        d.path(&["letterSpacing", "value"]).and_then(Val::as_f32),
        Some(2.0)
    );
    assert_eq!(d.text("textAlignVertical"), Some("CENTER"));
    assert_eq!(d.text("textDecoration"), Some("UNDERLINE"));
    assert_eq!(d.text("textCase"), Some("UPPER"));
    assert_eq!(d.text("textTruncation"), Some("ENDING"));
    assert_eq!(d.num("paragraphSpacing"), Some(8.0));
    let runs = d
        .path(&["textData", "styleOverrideTable"])
        .map(Val::items)
        .unwrap_or_default();
    assert!(!runs.is_empty());
    assert!(
        runs.iter().all(|r| r.get("letterSpacing").is_none()),
        "a run kept its letter spacing"
    );
    assert!(
        runs.iter().any(|r| r.get("fontSize").is_some()),
        "an unrelated run setting was dropped"
    );
    assert!(d.get("derivedTextData").is_none());

    // Text settings on a shape are refused.
    let (mut doc, mut h, rect) = rect();
    let op: EditOp = serde_json::from_value(serde_json::json!({
        "op": "set_typography", "target": rect.to_string(), "case": "upper"
    }))
    .unwrap();
    assert!(run(&mut doc, &mut h, vec![op]).is_err());
}

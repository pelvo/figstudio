//! Isolated integration test verifying that rejected enum strings and `PatchRaw` keys
//! never consume the process-wide schema identifier interning budget.

use std::path::PathBuf;

use figstudio_fig::Document;
use figstudio_fig::edit::text_runs::TextStyleRunPatch;
use figstudio_fig::edit::{EditOp, History, execute_edit_ops};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[test]
fn rejected_enum_and_patch_raw_edits_do_not_exhaust_schema_interner_budget() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(10);

    let created = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::CreateShape {
                id: None,
                kind: "FRAME".into(),
                parent: page,
                name: Some("Box".into()),
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 40.0,
                corner_radius: 0.0,
                fill_hex: None,
                count: None,
                star_inner_scale: None,
            },
            EditOp::CreateText {
                id: None,
                parent: page,
                x: 0.0,
                y: 50.0,
                characters: "Hello".into(),
                font_family: "Inter".into(),
                font_style: "Regular".into(),
                font_size: 14.0,
                fill_hex: "#111111".into(),
            },
        ],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids;
    let frame = created[0];
    let text = created[1];

    // Each payload is 4 MiB; 12 rejected payloads = 48 MiB > 32 MiB process interner cap.
    let big = |tag: &str| format!("{tag}_{}", "X".repeat(4 * 1024 * 1024));

    let patch_bad_top: EditOp = serde_json::from_value(serde_json::json!({
        "op": "patch_raw",
        "target": frame.to_string(),
        "set": { big("top"): true }
    }))
    .unwrap();
    let patch_bad_nested: EditOp = serde_json::from_value(serde_json::json!({
        "op": "patch_raw",
        "target": frame.to_string(),
        "set": { "size": { big("nested"): 1.0 } }
    }))
    .unwrap();

    let bad_ops = vec![
        EditOp::SetLayerProps {
            target: frame,
            visible: None,
            locked: None,
            opacity: None,
            blend_mode: Some(big("blend")),
            clip_content: None,
        },
        EditOp::SetConstraints {
            target: frame,
            horizontal: Some(big("hc")),
            vertical: Some(big("vc")),
        },
        EditOp::SetAutoLayout {
            target: frame,
            mode: big("mode"),
            spacing: None,
            counter_spacing: None,
            padding: None,
            primary_sizing: Some(big("ps")),
            counter_sizing: Some(big("cs")),
            primary_align: Some(big("pa")),
            counter_align: Some(big("ca")),
            wrap: None,
        },
        EditOp::SetAutoLayoutChild {
            target: frame,
            primary_fill: None,
            counter_stretch: None,
            positioning: Some(big("pos")),
            min_size: None,
            max_size: None,
        },
        EditOp::SetStroke {
            target: frame,
            hex: Some("#000000".into()),
            weight: Some(1.0),
            align: Some(big("align")),
        },
        EditOp::SetTypography {
            target: text,
            font_family: None,
            font_style: None,
            font_size: None,
            align_horizontal: Some(big("tah")),
            auto_resize: Some(big("tar")),
        },
        EditOp::CreateShape {
            id: None,
            kind: big("kind"),
            parent: page,
            name: None,
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        },
        EditOp::SetImageFill {
            target: frame,
            sha1_hex: "a".repeat(40),
            scale_mode: big("scale"),
            opacity: 1.0,
        },
        EditOp::StyleTextRange {
            target: text,
            utf16_start: 0,
            utf16_end: 5,
            style: TextStyleRunPatch {
                text_decoration: Some(big("dec")),
                text_case: Some(big("case")),
                ..Default::default()
            },
        },
        patch_bad_top,
        patch_bad_nested,
    ];

    let rev_before = doc.revision;
    for op in bad_ops {
        assert!(
            execute_edit_ops(&mut doc, &mut history, &[op], None, None, false).is_err(),
            "invalid enum or raw key must fail"
        );
        assert_eq!(doc.revision, rev_before);
    }

    // Opening another fixture must still succeed (schema identifier budget untouched).
    Document::open(fixture("vector.fig")).expect("schema interner budget must not be exhausted");
}

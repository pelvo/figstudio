//! End-to-end integration tests for Phase 4–10 (`EditOp`, Auto-Layout derive pass,
//! Rich Text Runs, Components/Overrides/Detach, Styles, Images, and Boolean Operations).

use std::path::PathBuf;

use figstudio_fig::Document;
use figstudio_fig::edit::text_runs::TextStyleRunPatch;
use figstudio_fig::edit::vector_net::BooleanOp;
use figstudio_fig::edit::{EditOp, History, execute_edit_ops, execute_import_image};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[test]
fn end_to_end_batch_creates_autolayout_card_with_rich_text_component_and_undo() {
    let mut doc = Document::open(fixture("stacks_wrap.fig")).expect("open stacks_wrap.fig");
    let baseline = doc.encode_message().unwrap();
    let page = doc.pages[0];
    let mut history = History::new(50);

    // 1. Dry-run first: must leave document untouched while reporting created IDs!
    let create_ops = vec![EditOp::CreateShape {
        id: None,
        kind: "FRAME".into(),
        parent: page,
        name: Some("Card".into()),
        x: 50.0,
        y: 80.0,
        width: 320.0,
        height: 40.0,
        corner_radius: 12.0,
        fill_hex: Some("#FFFFFF".into()),
        count: None,
        star_inner_scale: None,
    }];
    let (dry, _) =
        execute_edit_ops(&mut doc, &mut history, &create_ops, Some("Dry"), None, true).unwrap();
    assert!(!dry.applied);
    assert_eq!(dry.created_ids.len(), 1);
    assert_eq!(doc.encode_message().unwrap(), baseline);

    // 2. Commit frame + configure Auto-Layout + add child icon & rich text in later batch.
    let (rep1, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &create_ops,
        Some("Create Card"),
        None,
        false,
    )
    .unwrap();
    let card_id = rep1.created_ids[0];

    let (rep2, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::SetAutoLayout {
                target: card_id,
                mode: "HORIZONTAL".into(),
                spacing: Some(16.0),
                counter_spacing: None,
                padding: Some([12.0, 16.0, 12.0, 16.0]),
                primary_sizing: Some("FIXED".into()),
                counter_sizing: Some("HUG".into()),
                primary_align: Some("MIN".into()),
                counter_align: Some("CENTER".into()),
                wrap: Some(false),
            },
            EditOp::CreateShape {
                id: None,
                kind: "ROUNDED_RECTANGLE".into(),
                parent: card_id,
                name: Some("Badge".into()),
                x: 0.0,
                y: 0.0,
                width: 32.0,
                height: 32.0,
                corner_radius: 8.0,
                fill_hex: Some("#2563EB".into()),
                count: None,
                star_inner_scale: None,
            },
            EditOp::CreateText {
                id: None,
                parent: card_id,
                x: 0.0,
                y: 0.0,
                characters: "Hello Figma".into(),
                font_family: "Inter".into(),
                font_style: "Regular".into(),
                font_size: 16.0,
                fill_hex: "#111827".into(),
            },
        ],
        Some("Populate Card"),
        None,
        false,
    )
    .unwrap();

    let badge_id = rep2.created_ids[0];
    let text_id = rep2.created_ids[1];

    // Auto-Layout derive pass automatically placed Badge at x=16 and Text at x=16+32+16=64,
    // and hugged Card height to 12 + 32 + 12 = 56!
    let card_h = doc
        .node(card_id)
        .unwrap()
        .data
        .path(&["size", "y"])
        .unwrap()
        .as_f32()
        .unwrap();
    assert!(
        (card_h - 56.0).abs() < 1e-2,
        "expected card HUG height 56, got {card_h}"
    );
    let text_x = doc
        .node(text_id)
        .unwrap()
        .data
        .path(&["transform", "m02"])
        .unwrap()
        .as_f32()
        .unwrap();
    assert!(
        (text_x - 64.0).abs() < 1e-2,
        "expected text x=64, got {text_x}"
    );

    // 3. Apply rich text style run + fill style + convert card to component + instantiate & override & detach!
    let (rep3, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::StyleTextRange {
                target: text_id,
                utf16_start: 6,
                utf16_end: 11,
                style: TextStyleRunPatch {
                    font_style: Some("Bold".into()),
                    fill_hex: Some("#DC2626".into()),
                    ..Default::default()
                },
            },
            EditOp::CreateFillStyle {
                name: "Brand/Primary".into(),
                hex: "#10B981".into(),
            },
        ],
        Some("Style text and create fill style"),
        None,
        false,
    )
    .unwrap();
    let style_id = rep3.created_ids[0];

    let (rep4, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::BindStyle {
                target: badge_id,
                slot: "fill".into(),
                style: Some(style_id),
            },
            EditOp::CreateComponent { frame: card_id },
            EditOp::InstantiateComponent {
                symbol: card_id,
                parent: page,
                x: 50.0,
                y: 180.0,
            },
        ],
        Some("Componentize & instantiate"),
        None,
        false,
    )
    .unwrap();
    let inst_id = rep4.created_ids[0];
    assert_eq!(doc.node(card_id).unwrap().kind, "SYMBOL");
    assert_eq!(doc.node(inst_id).unwrap().kind, "INSTANCE");

    // Override text inside instance, then detach instance into a standalone FRAME!
    let text_ov_key = doc.node(text_id).unwrap().override_key();
    let (rep5, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::SetInstanceOverride {
                instance: inst_id,
                override_path: vec![text_ov_key],
                characters: Some("Overridden Copy".into()),
                visible: None,
                fill_hex: None,
                opacity: None,
            },
            EditOp::DetachInstance { instance: inst_id },
        ],
        Some("Override & detach"),
        None,
        false,
    )
    .unwrap();
    let detached_id = rep5.created_ids[0];
    assert!(doc.node(inst_id).is_none());
    assert_eq!(doc.node(detached_id).unwrap().kind, "FRAME");
    let detached_kids = doc.children(detached_id);
    assert_eq!(detached_kids.len(), 2);

    // 4. Image import + Boolean combine + round-trip encode/decode verification!
    let (img_hash, _) =
        execute_import_image(&mut doc, &mut history, b"\x89PNG\r\n\x1a\nfake".to_vec()).unwrap();
    let (rep6, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::CreateShape {
                id: None,
                kind: "ELLIPSE".into(),
                parent: page,
                name: Some("Circle A".into()),
                x: 400.0,
                y: 80.0,
                width: 80.0,
                height: 80.0,
                corner_radius: 0.0,
                fill_hex: Some("#F59E0B".into()),
                count: None,
                star_inner_scale: None,
            },
            EditOp::CreateShape {
                id: None,
                kind: "ELLIPSE".into(),
                parent: page,
                name: Some("Circle B".into()),
                x: 440.0,
                y: 80.0,
                width: 80.0,
                height: 80.0,
                corner_radius: 0.0,
                fill_hex: Some("#EF4444".into()),
                count: None,
                star_inner_scale: None,
            },
        ],
        Some("Circles"),
        None,
        false,
    )
    .unwrap();
    let (ca, cb) = (rep6.created_ids[0], rep6.created_ids[1]);
    let (rep7, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[
            EditOp::SetImageFill {
                target: ca,
                sha1_hex: img_hash,
                scale_mode: "FILL".into(),
                opacity: 1.0,
            },
            EditOp::BooleanCombine {
                operation: BooleanOp::Union,
                a: ca,
                b: cb,
                name: Some("Merged Circles".into()),
            },
        ],
        Some("Boolean union"),
        None,
        false,
    )
    .unwrap();
    assert_eq!(
        doc.node(rep7.created_ids[0]).unwrap().kind,
        "BOOLEAN_OPERATION"
    );

    // Verify full `.fig` archive serialization + re-open succeeds cleanly!
    let saved_bytes = doc.to_bytes().expect("to_bytes after complex edits");
    let reopened = Document::from_bytes(&saved_bytes).expect("reopen edited .fig");
    assert!(reopened.node(detached_id).is_some());

    // Undo every single step -> byte-for-byte identical to the original document!
    while history.undo(&mut doc).unwrap().is_some() {}
    assert_eq!(doc.encode_message().unwrap(), baseline);
}

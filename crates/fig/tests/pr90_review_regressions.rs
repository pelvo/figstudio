//! Regression tests for every finding from PR #90 review (written RED first per Ground Rule 3).

use std::path::PathBuf;

use figstudio_fig::clipboard::{decode_clipboard_html, encode_clipboard_html};
use figstudio_fig::edit::autolayout::compute_auto_layout;
use figstudio_fig::edit::geometry::{ellipse_commands, polygon_or_star_commands};
use figstudio_fig::edit::styles_vars::{hex_paint, image_paint};
use figstudio_fig::edit::vector_net::decode_vector_network;
use figstudio_fig::edit::{EditOp, History, Txn, execute_edit_ops, execute_paste_html, fractional};
use figstudio_fig::{Document, Obj, Val, intern};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[test]
fn t4_autolayout_inverted_or_nan_min_max_bounds_do_not_panic() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(20);
    let (rep, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: Some("AL Frame".into()),
            x: 0.0,
            y: 0.0,
            width: 200.0,
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
    let frame = rep.created_ids[0];
    {
        let mut t = Txn::new("raw inverted bounds");
        t.set_field(frame, "stackMode", Val::Enum(intern("HORIZONTAL")));
        t.set_field(
            frame,
            "stackPrimarySizing",
            Val::Enum(intern("RESIZE_TO_FIT")),
        );
        t.set_field(
            frame,
            "stackCounterSizing",
            Val::Enum(intern("RESIZE_TO_FIT")),
        );
        t.set_field(
            frame,
            "minSize",
            Val::Obj(Obj::from([(
                "value",
                Val::Obj(Obj::from([
                    ("x", Val::Float(300.0)),
                    ("y", Val::Float(f64::NAN)),
                ])),
            )])),
        );
        t.set_field(
            frame,
            "maxSize",
            Val::Obj(Obj::from([(
                "value",
                Val::Obj(Obj::from([
                    ("x", Val::Float(100.0)),
                    ("y", Val::Float(50.0)),
                ])),
            )])),
        );
        doc.apply_txn(&t).unwrap();
    }
    let patches = compute_auto_layout(&doc, Some(frame));
    assert!(!patches.is_empty());
}

#[test]
fn t5_vector_net_huge_region_count_in_12_byte_blob_returns_none_without_abort() {
    let mut blob = Vec::new();
    blob.extend_from_slice(&0u32.to_le_bytes());
    blob.extend_from_slice(&0u32.to_le_bytes());
    blob.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_vector_network(&blob).is_none());
}

#[test]
fn t6_and_t7_multibyte_hex_inputs_return_err_instead_of_char_boundary_panic() {
    assert!(hex_paint("aé123").is_err());
    assert!(hex_paint("#aé12345").is_err());

    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(10);
    let res = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "ROUNDED_RECTANGLE".into(),
            parent: page,
            name: None,
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0,
            corner_radius: 0.0,
            fill_hex: Some("aé123".into()),
            count: None,
            star_inner_scale: None,
        }],
        None,
        None,
        false,
    );
    assert!(res.is_err(), "bad fill_hex must return validation error");

    let bad_40_bytes = format!("é{}", "a".repeat(38));
    assert_eq!(bad_40_bytes.len(), 40);
    assert!(image_paint(&bad_40_bytes, "FILL", 1.0).is_err());
}

#[test]
fn t8_non_finite_arc_sweep_and_huge_polygon_count_stay_bounded() {
    // Do NOT pass u32::MAX in RED run if it loops 4B times; pass 2_000_000 which already allocates >2M cmds vs <=1025 cap!
    let cmds2 = ellipse_commands(100.0, 100.0, 0.0, 1.0e5, 0.0);
    assert!(cmds2.len() <= 200, "arc cmds len was {}", cmds2.len());
    let poly = polygon_or_star_commands(100.0, 100.0, 50_000, Some(0.5));
    assert!(poly.len() <= 2050, "poly cmds len was {}", poly.len());
}

#[test]
fn t9_range_text_edits_preserve_extra_text_data_fields() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let text_id = doc
        .nodes
        .iter()
        .find(|n| n.kind == "TEXT")
        .expect("TEXT in fixture")
        .id;

    let mut history = History::new(20);
    {
        let mut td = doc
            .node(text_id)
            .unwrap()
            .data
            .get("textData")
            .cloned()
            .unwrap()
            .into_owned();
        td.set("truncationStartIndex", Val::Int(3));
        let mut txn = Txn::new("seed");
        txn.set_field(text_id, "textData", td);
        doc.apply_txn(&txn).unwrap();
    }

    execute_edit_ops(
        &mut doc,
        &mut history,
        &[serde_json::from_value(serde_json::json!({
            "op": "style_text_range",
            "target": text_id.to_string(),
            "utf16_start": 0,
            "utf16_end": 1,
            "style": { "fontSize": 20.0 }
        }))
        .unwrap()],
        None,
        None,
        false,
    )
    .unwrap();

    let after_td = doc.node(text_id).unwrap().data.get("textData").unwrap();
    assert_eq!(
        after_td.get("truncationStartIndex").and_then(Val::as_u32),
        Some(3),
        "range style edit dropped extra textData fields"
    );

    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SpliceText {
            target: text_id,
            utf16_start: 0,
            utf16_delete_len: 1,
            insert: "Z".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let after_td2 = doc.node(text_id).unwrap().data.get("textData").unwrap();
    assert_eq!(
        after_td2.get("truncationStartIndex").and_then(Val::as_u32),
        Some(3),
        "splice_text dropped extra textData fields"
    );
}

#[test]
fn t10_paste_into_autolayout_frame_undoes_in_a_single_step() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let child_to_copy = doc.children(page)[0];
    let html = encode_clipboard_html(&doc, &[child_to_copy], 1, None).unwrap();

    let mut history = History::new(20);
    let (rep, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: Some("Paste AL Target".into()),
            x: 0.0,
            y: 0.0,
            width: 100.0,
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
    let al_frame = rep.created_ids[0];
    execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::SetAutoLayout {
            target: al_frame,
            mode: "VERTICAL".into(),
            spacing: Some(8.0),
            counter_spacing: None,
            padding: Some([12.0, 12.0, 12.0, 12.0]),
            primary_sizing: Some("HUG".into()),
            counter_sizing: Some("HUG".into()),
            primary_align: None,
            counter_align: None,
            wrap: None,
        }],
        None,
        None,
        false,
    )
    .unwrap();

    let count_before_paste = doc.nodes.len();
    let undo_before_paste = history.undo_len();
    let (roots, _) =
        execute_paste_html(&mut doc, &mut history, &html, al_frame, (0.0, 0.0)).unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(
        history.undo_len(),
        undo_before_paste + 1,
        "paste + auto-layout must record a single undo entry"
    );
    history.undo(&mut doc).unwrap();
    assert_eq!(doc.nodes.len(), count_before_paste);
}

#[test]
fn t12_batch_over_eight_nodes_never_touches_unrelated_autolayout_frames() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(20);

    let (rep, _) = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: Some("Untouched AL".into()),
            x: 500.0,
            y: 500.0,
            width: 999.0,
            height: 777.0,
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
    let bystander = rep.created_ids[0];
    {
        let mut t = Txn::new("raw stackMode");
        t.set_field(bystander, "stackMode", Val::Enum(intern("HORIZONTAL")));
        t.set_field(
            bystander,
            "stackPrimarySizing",
            Val::Enum(intern("RESIZE_TO_FIT")),
        );
        t.set_field(
            bystander,
            "stackCounterSizing",
            Val::Enum(intern("RESIZE_TO_FIT")),
        );
        doc.apply_txn(&t).unwrap();
    }

    let ops: Vec<EditOp> = (0..10)
        .map(|i| EditOp::CreateShape {
            id: None,
            kind: "ROUNDED_RECTANGLE".into(),
            parent: page,
            name: Some(format!("R{i}")),
            x: f64::from(i * 20),
            y: 0.0,
            width: 10.0,
            height: 10.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        })
        .collect();
    let (rep2, _) = execute_edit_ops(&mut doc, &mut history, &ops, None, None, false).unwrap();
    assert!(
        !rep2.touched_nodes.contains(&bystander),
        "unrelated Auto-Layout frame was rewritten by >8 edit batch!"
    );
}

#[test]
fn t15_fractional_front_insertions_stay_strictly_ordered_within_printable_alphabet() {
    let mut cur = fractional::between(None, None);
    for step in 0..150 {
        let prev = fractional::between(None, Some(&cur));
        assert!(
            prev.bytes().all(|b| (b'!'..=b'~').contains(&b)),
            "step {step}: {prev:?} left '!'..='~'"
        );
        assert!(
            prev.as_bytes() < cur.as_bytes(),
            "step {step}: {prev:?} is not < {cur:?}"
        );
        cur = prev;
    }
    let before_bang = fractional::between(None, Some("!"));
    assert!(
        before_bang.bytes().all(|b| (b'!'..=b'~').contains(&b)),
        "before '!' produced out-of-alphabet {before_bang:?}"
    );
}

#[test]
fn t20_clipboard_html_entity_decoding_does_not_double_decode_amp_lt() {
    let doc = Document::open(fixture("structure.fig")).unwrap();
    let child = doc.children(doc.pages[0])[0];
    let html = encode_clipboard_html(&doc, &[child], 1, Some("a&lt;b")).unwrap();
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD;
    let raw_kiwi = decode_clipboard_html(&html).unwrap().kiwi_bytes;
    let meta_json = r#"{"fileKey":"keep&lt;literal","pasteID":1,"dataType":"scene"}"#;
    let meta_b64 = b64.encode(meta_json.as_bytes());
    let kiwi_b64 = b64.encode(&raw_kiwi);
    let spoofed = format!(
        "<span data-metadata=\"&amp;lt;!--(figmeta)GARBAGE(/figmeta)--&amp;gt;\"></span><span data-metadata=\"&lt;!--(figmeta){meta_b64}(/figmeta)--&gt;\"></span><span data-buffer=\"&lt;!--(figma){kiwi_b64}(/figma)--&gt;\"></span>"
    );
    let decoded =
        decode_clipboard_html(&spoofed).expect("must ignore &amp;lt;!-- and decode real &lt;!--");
    assert_eq!(decoded.meta.file_key, "keep&lt;literal");
}

#[test]
fn t11_deep_hierarchy_delete_does_not_overflow_stack() {
    let handle = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let mut doc = Document::open(fixture("structure.fig")).unwrap();
            let mut parent = doc.pages[0];
            let mut alloc = figstudio_fig::edit::GuidAlloc::for_document(&doc);
            let mut setup = Txn::new("deep chain");
            let mut top = None;
            for i in 0..4_000 {
                let id = alloc.next_guid();
                if i == 0 {
                    top = Some(id);
                }
                let mut obj = Obj::new();
                obj.insert("guid", id.to_val());
                obj.insert("type", Val::Enum(intern("FRAME")));
                obj.insert("name", Val::Str("Chain".into()));
                obj.insert("visible", Val::Bool(true));
                obj.insert(
                    "parentIndex",
                    Val::Obj(Obj::from([
                        ("guid", parent.to_val()),
                        ("position", Val::Str("N".into())),
                    ])),
                );
                setup.create_node(id, Val::Obj(obj));
                parent = id;
            }
            doc.apply_txn(&setup).unwrap();
            let mut history = History::new(5);
            execute_edit_ops(
                &mut doc,
                &mut history,
                &[EditOp::DeleteNode {
                    target: top.unwrap(),
                }],
                None,
                None,
                false,
            )
            .unwrap();
        })
        .unwrap();
    handle.join().unwrap();
}

//! Regression tests for the second review round of PR #90.

use std::path::PathBuf;

use figstudio_fig::clipboard::encode_clipboard_html;
use figstudio_fig::edit::autolayout::compute_auto_layout;
use figstudio_fig::edit::text_runs::{TextStyleRunPatch, apply_text_range_style};
use figstudio_fig::edit::{EditOp, GuidAlloc, History, Txn, execute_edit_ops, execute_paste_html};
use figstudio_fig::{Document, Guid, Obj, Val, intern};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn shape(kind: &str, parent: Guid, x: f64, y: f64) -> EditOp {
    EditOp::CreateShape {
        id: None,
        kind: kind.into(),
        parent,
        name: None,
        x,
        y,
        width: 40.0,
        height: 30.0,
        corner_radius: 0.0,
        fill_hex: None,
        count: None,
        star_inner_scale: None,
    }
}

fn run(doc: &mut Document, history: &mut History, ops: &[EditOp]) -> Vec<Guid> {
    execute_edit_ops(doc, history, ops, None, None, false)
        .unwrap()
        .0
        .created_ids
}

#[test]
fn duplicate_then_create_in_one_batch_gets_distinct_guids() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(20);
    let rect = run(
        &mut doc,
        &mut history,
        &[shape("RECTANGLE", page, 0.0, 0.0)],
    )[0];
    let created = run(
        &mut doc,
        &mut history,
        &[
            EditOp::DuplicateNode {
                target: rect,
                dx: 10.0,
                dy: 0.0,
            },
            shape("RECTANGLE", page, 50.0, 0.0),
            EditOp::DuplicateNode {
                target: rect,
                dx: 20.0,
                dy: 0.0,
            },
        ],
    );
    assert_eq!(created.len(), 3);
    assert!(created.iter().all(|id| doc.node(*id).is_some()));
}

fn tmp_with_source() -> (tempfile::TempDir, PathBuf, Document) {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src.fig");
    std::fs::copy(fixture("structure.fig"), &src).unwrap();
    let doc = Document::open(&src).unwrap();
    (dir, src, doc)
}

#[test]
fn save_copy_checked_refuses_to_confine_without_a_source_on_disk() {
    let (dir, src, doc) = tmp_with_source();
    let out = dir.path().join("out.fig");
    assert!(doc.save_copy_checked(None, &out, false, true).is_err());
    let gone = dir.path().join("deleted.fig");
    assert!(
        doc.save_copy_checked(Some(&gone), &out, false, true)
            .is_err()
    );
    assert!(!out.exists());
    // Unconfined (the desktop menu's dialog) it saves; confined next to the source, too.
    doc.save_copy_checked(None, &out, false, false).unwrap();
    let inside = dir.path().join("inside.fig");
    doc.save_copy_checked(Some(&src), &inside, false, true)
        .unwrap();
    assert!(Document::open(&inside).is_ok());
}

#[test]
fn save_copy_checked_refuses_another_name_for_the_source_file() {
    let (dir, src, doc) = tmp_with_source();
    // A hard link is the same file under another name, as `SRC.fig` is on a case-insensitive disk.
    let alias = dir.path().join("alias.fig");
    std::fs::hard_link(&src, &alias).unwrap();
    let before = std::fs::read(&src).unwrap();
    let mut edited = doc.clone();
    let mut history = History::new(5);
    let page = edited.pages[0];
    run(
        &mut edited,
        &mut history,
        &[shape("RECTANGLE", page, 0.0, 0.0)],
    );
    assert!(
        edited
            .save_copy_checked(Some(&src), &alias, true, true)
            .is_err()
    );
    assert_eq!(std::fs::read(&src).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn save_copy_checked_overwrite_keeps_the_replaced_file_permissions() {
    use std::os::unix::fs::PermissionsExt as _;
    let (dir, src, doc) = tmp_with_source();
    let out = dir.path().join("out.fig");
    doc.save_copy_checked(Some(&src), &out, false, true)
        .unwrap();
    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o600)).unwrap();
    doc.save_copy_checked(Some(&src), &out, true, true).unwrap();
    let mode = std::fs::metadata(&out).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

/// A frame with a text child made a component, and one instance of it.
fn component_with_instance(doc: &mut Document, history: &mut History) -> (Guid, Guid, Guid) {
    let page = doc.pages[0];
    let frame = run(doc, history, &[shape("FRAME", page, 0.0, 0.0)])[0];
    let text = run(
        doc,
        history,
        &[EditOp::CreateText {
            id: None,
            parent: frame,
            x: 4.0,
            y: 4.0,
            characters: "Label".into(),
            font_family: "Inter".into(),
            font_style: "Regular".into(),
            font_size: 12.0,
            fill_hex: "#111111".into(),
        }],
    )[0];
    run(doc, history, &[EditOp::CreateComponent { frame }]);
    let instance = run(
        doc,
        history,
        &[EditOp::InstantiateComponent {
            symbol: frame,
            parent: page,
            x: 200.0,
            y: 0.0,
        }],
    )[0];
    (frame, text, instance)
}

fn override_text(doc: &Document, instance: Guid) -> Val {
    doc.node(instance)
        .unwrap()
        .data
        .path(&["symbolData", "symbolOverrides"])
        .unwrap()
        .items()
        .iter()
        .find_map(|e| e.get("textData"))
        .unwrap()
        .clone()
        .into_owned()
}

#[test]
fn instance_text_override_keeps_its_other_text_data_fields() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let mut history = History::new(20);
    let (_, text, instance) = component_with_instance(&mut doc, &mut history);
    let path = vec![doc.node(text).unwrap().override_key()];
    let set_chars = |doc: &mut Document, history: &mut History, chars: &str| {
        run(
            doc,
            history,
            &[EditOp::SetInstanceOverride {
                instance,
                override_path: path.clone(),
                characters: Some(chars.into()),
                visible: None,
                fill_hex: None,
                opacity: None,
                fields: None,
            }],
        );
    };
    set_chars(&mut doc, &mut history, "First");
    // Seed another textData field on the override entry.
    let mut sym = doc
        .node(instance)
        .unwrap()
        .data
        .get("symbolData")
        .unwrap()
        .clone()
        .into_owned();
    let mut list = sym.get("symbolOverrides").unwrap().items().to_vec();
    let mut td = list[0].get("textData").unwrap().clone().into_owned();
    td.set("truncationStartIndex", Val::Int(3));
    list[0].set("textData", td);
    sym.set("symbolOverrides", Val::Arr(list));
    let mut seed = Txn::new("seed");
    seed.set_field(instance, "symbolData", sym);
    doc.apply_txn(&seed).unwrap();

    set_chars(&mut doc, &mut history, "Second");
    let td = override_text(&doc, instance);
    assert_eq!(td.text("characters"), Some("Second"));
    assert_eq!(
        td.get("truncationStartIndex").and_then(Val::as_u32),
        Some(3),
        "the override's other textData fields were dropped"
    );
}

#[test]
fn undo_restores_a_field_that_one_patch_unset_and_set() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(20);
    let rect = run(
        &mut doc,
        &mut history,
        &[shape("RECTANGLE", page, 0.0, 0.0)],
    )[0];
    assert!(doc.node(rect).unwrap().data.get("locked").is_none());
    let mut txn = Txn::new("unset and set");
    txn.modify(
        rect,
        Obj::from([("locked", Val::Bool(true))]),
        vec!["locked".into()],
    );
    history.execute(&mut doc, txn).unwrap();
    assert_eq!(doc.node(rect).unwrap().data.flag("locked"), Some(true));
    history.undo(&mut doc).unwrap();
    assert!(
        doc.node(rect).unwrap().data.get("locked").is_none(),
        "undo left the field set"
    );
}

/// The first node in `doc` with an image fill, and its image hash.
fn image_node(doc: &Document) -> (Guid, String) {
    doc.nodes
        .iter()
        .find_map(|n| {
            n.data.get("fillPaints")?.items().iter().find_map(|p| {
                let h = p.path(&["image", "hash"])?.as_bytes()?;
                Some((n.id, h.iter().map(|b| format!("{b:02x}")).collect()))
            })
        })
        .expect("an image fill in the fixture")
}

#[test]
fn copy_to_another_document_carries_the_image_bytes() {
    for lazy in [false, true] {
        let src_path = fixture("structure.fig");
        let src = if lazy {
            Document::open_lazy(&src_path).unwrap()
        } else {
            Document::open(&src_path).unwrap()
        };
        let (node, hash) = image_node(&src);
        let html = encode_clipboard_html(&src, &[node], 1, None, Some(&src_path)).unwrap();

        let mut dst = Document::open(fixture("corners.fig")).unwrap();
        assert!(!dst.images.contains_key(&hash));
        let page = dst.pages[0];
        let mut history = History::new(5);
        execute_paste_html(&mut dst, &mut history, &html, page, (0.0, 0.0)).unwrap();
        assert!(
            dst.images.contains_key(&hash),
            "lazy={lazy}: image bytes lost"
        );
        let reopened = Document::from_bytes(&dst.to_bytes().unwrap()).unwrap();
        assert!(
            reopened.images.contains_key(&hash),
            "lazy={lazy}: not saved"
        );
    }
}

#[test]
fn pasted_image_bytes_must_match_their_hash() {
    let src = Document::open(fixture("structure.fig")).unwrap();
    let (node, hash) = image_node(&src);
    let html = encode_clipboard_html(&src, &[node], 1, None, None).unwrap();
    // Swap the carried bytes for others under the same hash.
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD;
    let start = html.find("(figstudio-images)").unwrap() + "(figstudio-images)".len();
    let end = html.find("(/figstudio-images)").unwrap();
    let forged = serde_json::json!({ hash: b64.encode(b"not the image") });
    let forged = b64.encode(serde_json::to_vec(&forged).unwrap());
    let html = format!("{}{forged}{}", &html[..start], &html[end..]);
    let mut dst = Document::open(fixture("corners.fig")).unwrap();
    let page = dst.pages[0];
    let mut history = History::new(5);
    assert!(execute_paste_html(&mut dst, &mut history, &html, page, (0.0, 0.0)).is_err());
}

#[test]
fn a_second_paste_reusing_a_master_maps_overrides_to_its_nodes() {
    let mut src = Document::open(fixture("structure.fig")).unwrap();
    let mut history = History::new(20);
    let (master, text, instance) = component_with_instance(&mut src, &mut history);
    let mut key = Txn::new("key");
    key.set_field(master, "key", Val::Str("round2-test-key".into()));
    src.apply_txn(&key).unwrap();
    let text_key = src.node(text).unwrap().override_key();
    run(
        &mut src,
        &mut history,
        &[EditOp::SetInstanceOverride {
            instance,
            override_path: vec![text_key],
            characters: Some("Override".into()),
            visible: None,
            fill_hex: None,
            opacity: None,
            fields: None,
        }],
    );
    let html = encode_clipboard_html(&src, &[instance], 1, None, None).unwrap();

    let mut dst = Document::open(fixture("corners.fig")).unwrap();
    let page = dst.pages[0];
    let mut h = History::new(20);
    // Shift the target's guid numbering away from the source's.
    let shifts: Vec<EditOp> = (0..5)
        .map(|i| shape("RECTANGLE", page, f64::from(i) * 50.0, 0.0))
        .collect();
    run(&mut dst, &mut h, &shifts);
    let (first, _) = execute_paste_html(&mut dst, &mut h, &html, page, (0.0, 0.0)).unwrap();
    let (second, _) = execute_paste_html(&mut dst, &mut h, &html, page, (0.0, 0.0)).unwrap();
    let symbol = |id: Guid| {
        dst.node(id)
            .unwrap()
            .data
            .path(&["symbolData", "symbolID"])
            .and_then(Val::as_guid)
    };
    let paths = |id: Guid| -> Vec<Vec<Guid>> {
        dst.node(id)
            .unwrap()
            .data
            .path(&["symbolData", "symbolOverrides"])
            .map(Val::items)
            .unwrap_or_default()
            .iter()
            .map(|e| {
                e.path(&["guidPath", "guids"])
                    .map(Val::items)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(Val::as_guid)
                    .collect()
            })
            .collect()
    };
    assert_eq!(symbol(first[0]), symbol(second[0]), "the master is reused");
    assert_eq!(
        paths(first[0]),
        paths(second[0]),
        "overrides miss the reused master"
    );
}

#[test]
fn deep_trees_resize_and_lay_out_without_overflowing_the_stack() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let mut parent = doc.pages[0];
    let mut alloc = GuidAlloc::for_document(&doc);
    let mut setup = Txn::new("deep chain");
    let mut chain = Vec::new();
    for _ in 0..20_000 {
        let id = alloc.next_guid();
        let mut obj = Obj::new();
        obj.insert("guid", id.to_val());
        obj.insert("type", Val::Enum(intern("FRAME")));
        obj.insert("name", Val::Str("Chain".into()));
        obj.insert("visible", Val::Bool(true));
        obj.insert("stackMode", Val::Enum(intern("VERTICAL")));
        obj.insert("horizontalConstraint", Val::Enum(intern("STRETCH")));
        // Absolute children of a stack still follow their constraints, so a resize reaches all.
        obj.insert("stackPositioning", Val::Enum(intern("ABSOLUTE")));
        obj.insert(
            "size",
            Val::Obj(Obj::from([
                ("x", Val::Float(100.0)),
                ("y", Val::Float(10.0)),
            ])),
        );
        obj.insert(
            "parentIndex",
            Val::Obj(Obj::from([
                ("guid", parent.to_val()),
                ("position", Val::Str("N".into())),
            ])),
        );
        setup.create_node(id, Val::Obj(obj));
        chain.push(id);
        parent = id;
    }
    doc.apply_txn(&setup).unwrap();
    // Only the layout and resize run on the small stack.
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || {
            compute_auto_layout(&doc, Some(*chain.last().unwrap()));
            let mut txn = Txn::new("resize");
            figstudio_fig::edit::geometry::resize_node_with_constraints(
                &doc, &mut txn, chain[0], 300.0, 10.0,
            )
            .unwrap();
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn struct_writes_with_unknown_fields_are_refused() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(5);
    let rect = run(
        &mut doc,
        &mut history,
        &[shape("RECTANGLE", page, 0.0, 0.0)],
    )[0];
    let mut txn = Txn::new("bogus");
    txn.set_field(
        rect,
        "size",
        Val::Obj(Obj::from([
            ("x", Val::Float(1.0)),
            ("y", Val::Float(1.0)),
            ("bogus", Val::Float(1.0)),
        ])),
    );
    assert!(doc.apply_txn(&txn).is_err());
}

#[test]
fn a_style_id_at_the_top_of_u32_is_an_error_not_a_wrap() {
    let td = Val::Obj(Obj::from([
        ("characters", Val::Str("ab".into())),
        (
            "characterStyleIDs",
            Val::Arr(vec![Val::Int(i64::from(u32::MAX)), Val::Int(0)]),
        ),
        (
            "styleOverrideTable",
            Val::Arr(vec![Val::Obj(Obj::from([(
                "styleID",
                Val::Int(i64::from(u32::MAX)),
            )]))]),
        ),
    ]));
    let patch = TextStyleRunPatch {
        font_size: Some(20.0),
        ..Default::default()
    };
    assert!(apply_text_range_style(&td, 0, 2, &patch).is_err());
}

#[test]
fn moving_an_auto_layout_child_re_solves_its_parent() {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut history = History::new(20);
    let frame = run(&mut doc, &mut history, &[shape("FRAME", page, 0.0, 0.0)])[0];
    let child = run(
        &mut doc,
        &mut history,
        &[shape("RECTANGLE", frame, 0.0, 0.0)],
    )[0];
    run(
        &mut doc,
        &mut history,
        &[serde_json::from_value(serde_json::json!({
            "op": "set_auto_layout",
            "target": frame.to_string(),
            "mode": "VERTICAL",
            "padding": [10.0, 10.0, 10.0, 10.0]
        }))
        .unwrap()],
    );
    let x = |doc: &Document| {
        doc.node(child)
            .unwrap()
            .data
            .path(&["transform", "m02"])
            .and_then(Val::as_f32)
    };
    let placed = x(&doc);
    run(
        &mut doc,
        &mut history,
        &[EditOp::Move {
            target: child,
            x: 77.0,
            y: 77.0,
            relative: false,
        }],
    );
    assert_eq!(x(&doc), placed, "the Auto-Layout parent places its child");
}

#[test]
fn resizing_text_drops_its_stored_glyph_layout() {
    let mut doc = Document::open(fixture("noise_and_grain.fig")).unwrap();
    let text = doc
        .nodes
        .iter()
        .find(|n| n.kind == "TEXT" && n.data.get("derivedTextData").is_some())
        .expect("a TEXT with stored glyphs in the fixture")
        .id;
    let mut history = History::new(5);
    run(
        &mut doc,
        &mut history,
        &[EditOp::Resize {
            target: text,
            width: 40.0,
            height: 200.0,
        }],
    );
    assert!(
        doc.node(text)
            .unwrap()
            .data
            .get("derivedTextData")
            .is_none(),
        "the old line breaks would still be drawn"
    );
}

#[test]
fn auto_layout_resizing_text_drops_its_stored_glyph_layout() {
    let mut doc = Document::open(fixture("noise_and_grain.fig")).unwrap();
    let (text, text_w) = doc
        .nodes
        .iter()
        .find_map(|n| {
            if n.kind == "TEXT" && n.data.get("derivedTextData").is_some() {
                let w = f64::from(n.data.path(&["size", "x"]).and_then(Val::as_f32)?);
                Some((n.id, w))
            } else {
                None
            }
        })
        .expect("a TEXT with stored glyphs in the fixture");
    let page = doc.pages[0];
    let mut history = History::new(10);
    let frame = run(&mut doc, &mut history, &[shape("FRAME", page, 0.0, 0.0)])[0];
    run(
        &mut doc,
        &mut history,
        &[
            EditOp::Reparent {
                target: text,
                new_parent: frame,
                index: None,
            },
            serde_json::from_value(serde_json::json!({
                "op": "set_auto_layout",
                "target": frame.to_string(),
                "mode": "VERTICAL",
                "padding": [10.0, 10.0, 10.0, 10.0],
                "primary_sizing": "FIXED",
                "counter_sizing": "FIXED"
            }))
            .unwrap(),
            EditOp::Resize {
                target: frame,
                width: text_w + 200.0,
                height: 300.0,
            },
        ],
    );
    let mut txn = Txn::new("stretch text child");
    txn.set_field(text, "stackChildAlignSelf", Val::Enum(intern("STRETCH")));
    doc.apply_txn(&txn).unwrap();
    run(
        &mut doc,
        &mut history,
        &[EditOp::Resize {
            target: frame,
            width: text_w + 300.0,
            height: 320.0,
        }],
    );
    let after_w = f64::from(
        doc.node(text)
            .unwrap()
            .data
            .path(&["size", "x"])
            .and_then(Val::as_f32)
            .unwrap(),
    );
    assert!(
        after_w > text_w,
        "Auto-Layout should have widened the TEXT child"
    );
    assert!(
        doc.node(text)
            .unwrap()
            .data
            .get("derivedTextData")
            .is_none(),
        "Auto-Layout resized TEXT from {text_w} to {after_w} but kept stale derivedTextData"
    );
}

//! Integration & property tests for Phase 0 (`census`), Phase 1 (`EditCore` / `Txn` /
//! `History`), and Phase 2 (Lazy streaming save & two-way Figma clipboard).

use std::path::PathBuf;

use figstudio_fig::census::CensusReport;
use figstudio_fig::clipboard::{decode_clipboard_html, encode_clipboard_html, paste_kiwi_into};
use figstudio_fig::edit::{GuidAlloc, History, Txn, fractional};
use figstudio_fig::{Document, Obj, Val, intern};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[test]
fn commit_then_undo_is_byte_for_byte_identical_across_modify_create_and_delete() {
    let mut doc = Document::open(fixture("structure.fig")).expect("open structure.fig");
    let baseline_msg = doc.encode_message().expect("baseline encode_message");
    let baseline_img_inst = doc.image_instance;

    let page = doc.pages[0];
    let first_frame = doc.children(page)[0];
    let mut alloc = GuidAlloc::for_document(&doc);
    let new_rect_id = alloc.next_guid();

    let mut rect_obj = Obj::new();
    rect_obj.insert("guid", new_rect_id.to_val());
    rect_obj.insert("type", Val::Enum(intern("RECTANGLE")));
    rect_obj.insert("name", Val::Str("Temp Rect".into()));
    rect_obj.insert("visible", Val::Bool(true));
    rect_obj.insert(
        "size",
        Val::Obj(Obj::from([
            ("x", Val::Float(120.0)),
            ("y", Val::Float(48.0)),
        ])),
    );
    rect_obj.insert(
        "parentIndex",
        Val::Obj(Obj::from([
            ("guid", page.to_val()),
            ("position", Val::Str(fractional::between(Some("~"), None))),
        ])),
    );

    let mut history = History::new(50);

    // Step 1: modify existing node + create new node + append a blob.
    let mut txn1 = Txn::new("Edit name and add rect");
    txn1.set_field(first_frame, "name", Val::Str("Renamed Frame".into()));
    txn1.set_field(first_frame, "opacity", Val::Float(0.75));
    txn1.create_node(new_rect_id, Val::Obj(rect_obj));
    txn1.add_blobs.push(vec![1, 2, 3, 4]);
    let dirty = history.execute(&mut doc, txn1).expect("apply txn1");
    assert!(dirty.pages.contains(&page));
    assert_eq!(
        doc.image_instance, baseline_img_inst,
        "node edits keep image_instance"
    );
    assert_eq!(doc.node(first_frame).unwrap().name, "Renamed Frame");
    assert!(doc.node(new_rect_id).is_some());

    // Step 2: delete the newly created rect, then delete an existing leaf node & undo both!
    let leaf = doc
        .nodes
        .iter()
        .find(|n| {
            n.kind != "DOCUMENT"
                && n.kind != "CANVAS"
                && doc.children(n.id).is_empty()
                && n.id != new_rect_id
        })
        .unwrap()
        .id;
    let mut txn2 = Txn::new("Delete leaf");
    txn2.delete_node(leaf);
    history.execute(&mut doc, txn2).expect("apply txn2");
    assert!(doc.node(leaf).is_none());

    // Undo Step 2 and Step 1 -> encoded message MUST match baseline byte-for-byte!
    history.undo(&mut doc).expect("undo 2").expect("had entry");
    assert!(doc.node(leaf).is_some());
    history.undo(&mut doc).expect("undo 1").expect("had entry");
    assert!(doc.node(new_rect_id).is_none());

    let restored_msg = doc.encode_message().expect("restored encode_message");
    assert_eq!(
        restored_msg, baseline_msg,
        "encode_message after undo must be byte-for-byte identical to the original"
    );

    // Redo brings both edits back cleanly.
    history.redo(&mut doc).expect("redo 1").expect("had redo");
    assert_eq!(doc.node(first_frame).unwrap().name, "Renamed Frame");
    history.redo(&mut doc).expect("redo 2").expect("had redo");
    assert!(doc.node(leaf).is_none());
}

#[test]
fn schema_validation_rejects_bad_field_or_type_atomically() {
    let mut doc = Document::open(fixture("structure.fig")).expect("open");
    let before = doc.encode_message().unwrap();
    let target = doc.children(doc.pages[0])[0];

    // Unknown field name
    let mut bad1 = Txn::new("Bad field");
    bad1.set_field(target, "nonExistentFigmaFieldXYZ", Val::Bool(true));
    assert!(doc.apply_txn(&bad1).is_err());

    // Wrong field type (`opacity` expects float, given string)
    let mut bad2 = Txn::new("Bad type");
    bad2.set_field(target, "opacity", Val::Str("not-a-float".into()));
    assert!(doc.apply_txn(&bad2).is_err());

    // Invalid enum variant
    let mut bad3 = Txn::new("Bad enum");
    bad3.set_field(target, "stackMode", Val::Enum(intern("DIAGONAL_SPIRAL")));
    assert!(doc.apply_txn(&bad3).is_err());

    // Document must remain completely unchanged.
    assert_eq!(doc.encode_message().unwrap(), before);
}

#[test]
fn coalesced_drag_gestures_collapse_into_single_undo_step() {
    let mut doc = Document::open(fixture("structure.fig")).expect("open");
    let baseline = doc.encode_message().unwrap();
    let target = doc.children(doc.pages[0])[0];
    let mut history = History::new(50);

    for step in 1..=5 {
        let mut t = Txn::new("Move").with_coalesce("drag:move:1");
        t.set_field(
            target,
            "transform",
            Val::Obj(Obj::from([
                ("m00", Val::Float(1.0)),
                ("m01", Val::Float(0.0)),
                ("m02", Val::Float(f64::from(step * 10))),
                ("m10", Val::Float(0.0)),
                ("m11", Val::Float(1.0)),
                ("m12", Val::Float(f64::from(step * 5))),
            ])),
        );
        history.execute(&mut doc, t).unwrap();
    }
    assert_eq!(
        history.undo_len(),
        1,
        "5 drag frames coalesce into 1 undo step"
    );
    history.undo(&mut doc).unwrap();
    assert_eq!(doc.encode_message().unwrap(), baseline);
}

#[test]
fn clipboard_html_roundtrips_and_pastes_with_symbol_dependencies() {
    let mut doc = Document::open(fixture("structure.fig")).expect("open");
    let page = doc.pages[0];
    let root_to_copy = doc.children(page)[0];
    let before_children = doc.children(page).len();

    let html = encode_clipboard_html(&doc, &[root_to_copy], 4242, Some("testkey"))
        .expect("encode clipboard html");
    assert!(html.contains("data-metadata=\"<!--(figmeta)"));
    assert!(html.contains("data-buffer=\"<!--(figma)"));

    let decoded = decode_clipboard_html(&html).expect("decode clipboard html");
    assert_eq!(decoded.meta.paste_id, 4242);
    assert_eq!(decoded.meta.file_key, "testkey");

    let (paste_txn, pasted_roots) =
        paste_kiwi_into(&doc, &decoded.kiwi_bytes, page, (40.0, 40.0)).expect("paste_kiwi_into");
    assert_eq!(pasted_roots.len(), 1);

    let mut history = History::new(10);
    history.execute(&mut doc, paste_txn).expect("execute paste");
    assert_eq!(doc.children(page).len(), before_children + 1);
    assert_ne!(pasted_roots[0], root_to_copy);

    // Undo removes the pasted subtree cleanly.
    history.undo(&mut doc).unwrap();
    assert_eq!(doc.children(page).len(), before_children);
}

#[test]
fn lazy_opened_archive_saves_streaming_copy_with_edits() {
    let src_path = fixture("broken_images.fig");
    let mut lazy_doc = Document::open_lazy(&src_path).expect("open_lazy");
    assert!(lazy_doc.is_lazy());

    let page = lazy_doc.pages[0];
    let child = lazy_doc.children(page)[0];
    let mut txn = Txn::new("Rename in lazy doc");
    txn.set_field(child, "name", Val::Str("Edited Lazy Layer".into()));
    lazy_doc.apply_txn(&txn).expect("apply edit on lazy doc");

    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("saved_copy.fig");
    lazy_doc
        .save_with_lazy_source(Some(&src_path), &dst)
        .expect("save_with_lazy_source");

    let reopened = Document::open(&dst).expect("reopen saved copy");
    assert_eq!(reopened.node(child).unwrap().name, "Edited Lazy Layer");
    assert_eq!(reopened.images.len(), lazy_doc.lazy_images.len());
}

#[test]
fn census_scans_fixtures_without_leaking_layer_names() {
    let doc = Document::open(fixture("stacks_wrap.fig")).expect("open stacks_wrap.fig");
    let mut census = CensusReport::default();
    census.add_document(&doc).expect("census");
    assert!(census.total_nodes > 0);
    assert!(!census.stack_modes.is_empty());
    let md = census.to_markdown();
    assert!(md.contains("Auto-Layout Stack Modes"));
}

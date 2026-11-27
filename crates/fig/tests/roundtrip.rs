use std::io::Read;
use std::path::PathBuf;

use figstudio_fig::{Document, Guid, Node, Val};

const FIXTURES: [&str; 10] = [
    "broken_images",
    "corners",
    "motion",
    "motion-probe",
    "noise_and_grain",
    "sections",
    "shaders",
    "stacks_wrap",
    "structure",
    "vector",
];

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(format!("{name}.fig"))
}

/// The decompressed Kiwi message straight from the file, independent of the decoder.
fn raw_message(file: &[u8]) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(file)).unwrap();
    let mut canvas = Vec::new();
    zip.by_name("canvas.fig")
        .unwrap()
        .read_to_end(&mut canvas)
        .unwrap();
    let mut at = 12;
    let mut chunks = Vec::new();
    while at + 4 <= canvas.len() {
        let len = u32::from_le_bytes(canvas[at..at + 4].try_into().unwrap()) as usize;
        chunks.push(canvas[at + 4..at + 4 + len].to_vec());
        at += 4 + len;
    }
    let chunk = &chunks[1];
    let mut out = Vec::new();
    if chunk.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        ruzstd::decoding::StreamingDecoder::new(chunk.as_slice())
            .unwrap()
            .read_to_end(&mut out)
            .unwrap();
    } else {
        flate2::read::DeflateDecoder::new(chunk.as_slice())
            .read_to_end(&mut out)
            .unwrap();
    }
    out
}

fn assert_same(name: &str, a: &Document, b: &Document) {
    assert_eq!(a.nodes.len(), b.nodes.len(), "{name}: node count");
    for (x, y) in a.nodes.iter().zip(&b.nodes) {
        assert_eq!(x.id, y.id, "{name}: node order");
        assert_eq!(x.data, y.data, "{name}: node {} differs", x.id);
    }
    assert_eq!(a.blobs, b.blobs, "{name}: blobs");
    assert_eq!(a.images, b.images, "{name}: images");
    assert_eq!(a.thumbnail, b.thumbnail, "{name}: thumbnail");
    assert_eq!(a.file_name, b.file_name, "{name}: file name");
    assert_eq!(a.pages, b.pages, "{name}: pages");
    assert_eq!(a.format_version, b.format_version, "{name}: version");
}

#[test]
fn every_fixture_survives_write_then_read() {
    for name in FIXTURES {
        let doc = Document::open(fixture(name)).unwrap();
        let bytes = doc.to_bytes().unwrap_or_else(|e| panic!("{name}: {e}"));
        let again = Document::from_bytes(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_same(name, &doc, &again);
        // Writing twice gives identical bytes.
        assert_eq!(
            bytes,
            again.to_bytes().unwrap(),
            "{name}: second write differs"
        );
    }
}

#[test]
fn encoded_message_matches_the_original_bytes() {
    for name in FIXTURES {
        let file = std::fs::read(fixture(name)).unwrap();
        let doc = Document::from_bytes(&file).unwrap();
        let ours = doc.encode_message().unwrap();
        assert!(
            ours == raw_message(&file),
            "{name}: message bytes differ ({} vs {})",
            ours.len(),
            raw_message(&file).len()
        );
    }
}

/// GUIDs decode inline (no heap object each) and still write back, also when an edit
/// supplies one as the two-field object a JSON round trip produces.
#[test]
fn guids_decode_inline_and_write_back_in_either_form() {
    let mut doc = Document::open(fixture("structure")).unwrap();
    let node = doc.node(Guid(1, 42)).unwrap();
    assert!(matches!(
        node.data.get("guid"),
        Some(Val::Guid(Guid(1, 42)))
    ));
    let parent = node.data.path(&["parentIndex", "guid"]).unwrap().clone();
    assert!(matches!(parent, Val::Guid(_)));
    assert_eq!(parent.as_guid(), node.parent);

    let as_obj: Val = serde_json::from_value(serde_json::to_value(&parent).unwrap()).unwrap();
    assert!(matches!(as_obj, Val::Obj(_)));
    assert_eq!(as_obj, parent);
    let mut nodes = doc.nodes.clone();
    let edited = nodes.iter_mut().find(|n| n.id == Guid(1, 42)).unwrap();
    let Some(Val::Obj(index)) = edited.data.get_mut("parentIndex") else {
        panic!()
    };
    index.insert("guid", as_obj);
    doc.set_nodes(nodes).unwrap();
    let again = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
    assert_eq!(again.node(Guid(1, 42)).unwrap().parent, parent.as_guid());
}

/// A field still encoded when opened (`derivedSymbolData`) is written as edited, not as its
/// original bytes, once changed through `get_mut`.
#[test]
fn an_edited_lazy_field_is_written_as_edited() {
    let mut doc = Document::open(fixture("structure")).unwrap();
    let mut nodes = doc.nodes.clone();
    let node = nodes
        .iter_mut()
        .find(|n| n.data.get("derivedSymbolData").is_some())
        .unwrap();
    let id = node.id;
    let Some(Val::Arr(items)) = node.data.get_mut("derivedSymbolData") else {
        panic!()
    };
    assert!(items.len() > 1);
    items.truncate(1);
    doc.set_nodes(nodes).unwrap();
    let again = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
    let list = again
        .node(id)
        .unwrap()
        .data
        .get("derivedSymbolData")
        .unwrap();
    assert_eq!(list.items().len(), 1);
}

/// An instance's override list, kept encoded inside `symbolData` until read, is written as
/// edited once changed, and other instances' unedited lists keep their bytes.
#[test]
fn an_edited_nested_lazy_field_is_written_as_edited() {
    let mut doc = Document::open(fixture("shaders")).unwrap();
    let overrides = |data: &Val| {
        data.path(&["symbolData", "symbolOverrides"])
            .map(|v| v.items().len())
    };
    let mut nodes = doc.nodes.clone();
    let mut with = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| overrides(&n.data).is_some_and(|len| len > 1))
        .map(|(i, n)| (i, n.id, overrides(&n.data)));
    let (i, edited, _) = with.next().unwrap();
    let (_, other, other_len) = with.next().unwrap();
    let Some(Val::Arr(list)) = nodes[i]
        .data
        .get_mut("symbolData")
        .and_then(|s| s.get_mut("symbolOverrides"))
    else {
        panic!()
    };
    list.truncate(1);
    doc.set_nodes(nodes).unwrap();
    let again = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
    assert_eq!(overrides(&again.node(edited).unwrap().data), Some(1));
    assert_eq!(overrides(&again.node(other).unwrap().data), other_len);
}

/// Two copies of a document compare equal without decoding their still-encoded fields
/// (as compare does for every unchanged node), and a real difference is still seen.
#[test]
fn equal_documents_compare_equal_without_decoding_their_lazy_fields() {
    let encoded = |doc: &Document| -> usize {
        doc.nodes
            .iter()
            .map(|n| match &n.data {
                Val::Obj(o) => o.len() - o.values_decoded().count(),
                _ => 0,
            })
            .sum()
    };
    let a = Document::open(fixture("structure")).unwrap();
    let mut b = Document::open(fixture("structure")).unwrap();
    let before = encoded(&a);
    assert!(before > 0);
    assert!(a.nodes.iter().zip(&b.nodes).all(|(x, y)| x.data == y.data));
    assert_eq!((encoded(&a), encoded(&b)), (before, before));

    let mut nodes = b.nodes.clone();
    let node = nodes
        .iter_mut()
        .find(|n| n.data.get("derivedSymbolData").is_some())
        .unwrap();
    let Some(Val::Arr(items)) = node.data.get_mut("derivedSymbolData") else {
        panic!()
    };
    items.truncate(1);
    b.set_nodes(nodes).unwrap();
    assert!(!a.nodes.iter().zip(&b.nodes).all(|(x, y)| x.data == y.data));
}

#[test]
fn edits_are_written() {
    let mut doc = Document::open(fixture("structure")).unwrap();
    let id = Guid(1, 42);
    let mut nodes = doc.nodes.clone();
    let node = nodes.iter_mut().find(|n| n.id == id).unwrap();
    let Val::Obj(map) = &mut node.data else {
        panic!()
    };
    map.insert("name", Val::Str("renamed layer".into()));
    map.insert("visible", Val::Bool(false));
    let edited = Node::from_data(node.data.clone()).unwrap();
    *node = edited;
    doc.set_nodes(nodes).unwrap();
    let again = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
    let n = again.node(id).unwrap();
    assert_eq!(n.name, "renamed layer");
    assert_eq!(n.data.flag("visible"), Some(false));
}

#[test]
fn unknown_fields_and_wrong_types_are_errors_not_panics() {
    let mut doc = Document::open(fixture("vector")).unwrap();
    let mut nodes = doc.nodes.clone();
    if let Val::Obj(map) = &mut nodes[3].data {
        map.insert("notAFigmaField", Val::Int(1));
    }
    doc.set_nodes(nodes.clone()).unwrap();
    let e = doc.to_bytes().unwrap_err();
    assert!(e.to_string().contains("notAFigmaField"), "{e}");

    if let Val::Obj(map) = &mut nodes[3].data {
        map.remove("notAFigmaField");
        map.insert("opacity", Val::Str("high".into()));
    }
    doc.set_nodes(nodes).unwrap();
    let e = doc.to_bytes().unwrap_err();
    assert!(e.to_string().contains("expected a number"), "{e}");
}

#[test]
fn bare_document_preserves_bare_format_when_asset_free_and_upgrades_to_zip_when_given_assets() {
    use figstudio_fig::clipboard::encode_clipboard_html;
    use figstudio_fig::edit::{
        EditOp, History, Txn, execute_edit_ops, execute_import_image, execute_paste_html,
    };

    let base = Document::open(fixture("structure")).unwrap();
    let bare_bytes = base.encode_canvas().unwrap();
    assert!(!bare_bytes.starts_with(b"PK"));

    // 1. Asset-free edit on a bare document stays bare fig-kiwi.
    let mut bare = Document::from_bytes(&bare_bytes).unwrap();
    let page = bare.pages[0];
    let target = bare.children(page)[0];
    let mut txn = Txn::new("rename bare");
    txn.set_field(target, "name", Val::Str("Bare Renamed".into()));
    bare.apply_txn(&txn).unwrap();
    let bare_out = bare.to_bytes().unwrap();
    assert!(
        bare_out.starts_with(b"fig-"),
        "asset-free bare document must stay bare fig-kiwi"
    );
    assert_eq!(
        Document::from_bytes(&bare_out)
            .unwrap()
            .node(target)
            .unwrap()
            .name,
        "Bare Renamed"
    );

    // 2. Importing an image and assigning a fill promotes bare output to a ZIP containing the image.
    let mut imported = Document::from_bytes(&bare_bytes).unwrap();
    let import_page = imported.pages[0];
    let mut history = History::new(8);
    let png = b"\x89PNG\r\n\x1a\nimported-bare-image".to_vec();
    let (img_hash, _) = execute_import_image(&mut imported, &mut history, png.clone()).unwrap();
    let created = execute_edit_ops(
        &mut imported,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "RECTANGLE".into(),
            parent: import_page,
            name: None,
            x: 0.0,
            y: 0.0,
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
    execute_edit_ops(
        &mut imported,
        &mut history,
        &[EditOp::SetImageFill {
            target: created,
            sha1_hex: img_hash.clone(),
            scale_mode: "FILL".into(),
            opacity: 1.0,
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let imported_out = imported.to_bytes().unwrap();
    assert!(
        imported_out.starts_with(b"PK"),
        "bare document with imported image must serialize as a ZIP archive"
    );
    let reopened_import = Document::from_bytes(&imported_out).unwrap();
    assert_eq!(reopened_import.images.get(&img_hash), Some(&png));

    // 3. Pasting clipboard HTML carrying an image into a bare document preserves the pasted image.
    let img_node = base
        .nodes
        .iter()
        .find(|n| {
            n.data
                .get("fillPaints")
                .map(Val::items)
                .is_some_and(|ps| ps.iter().any(|p| p.path(&["image", "hash"]).is_some()))
        })
        .unwrap()
        .id;
    let html = encode_clipboard_html(&base, &[img_node], 1, None, None).unwrap();
    let mut pasted = Document::from_bytes(&bare_bytes).unwrap();
    let paste_page = pasted.pages[0];
    let mut paste_history = History::new(4);
    execute_paste_html(
        &mut pasted,
        &mut paste_history,
        &html,
        paste_page,
        (0.0, 0.0),
    )
    .unwrap();
    assert!(!pasted.images.is_empty());
    let pasted_out = pasted.to_bytes().unwrap();
    assert!(pasted_out.starts_with(b"PK"));
    let reopened_paste = Document::from_bytes(&pasted_out).unwrap();
    assert_eq!(reopened_paste.images, pasted.images);

    // 4. Setting only a thumbnail on a bare document also preserves the thumbnail in a ZIP archive.
    let mut with_thumb = Document::from_bytes(&bare_bytes).unwrap();
    let thumb = b"\x89PNG\r\n\x1a\nbare-thumb".to_vec();
    with_thumb.thumbnail = Some(thumb.clone());
    let thumb_out = with_thumb.to_bytes().unwrap();
    assert!(thumb_out.starts_with(b"PK"));
    let reopened_thumb = Document::from_bytes(&thumb_out).unwrap();
    assert_eq!(reopened_thumb.thumbnail.as_deref(), Some(thumb.as_slice()));
}

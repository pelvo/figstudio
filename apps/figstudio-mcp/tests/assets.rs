//! `download_assets`: rendered formats, original image bytes, limits and per-node errors.
use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_fig::{Document, Guid, Node, Obj, Val, hex};
use figstudio_mcp::tools::assets::{AssetFormat, download};
use rmcp::model::CallToolResult;
use serde_json::Value;
use std::path::PathBuf;

fn fixture(name: &str) -> Document {
    Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../tests/fixtures/{name}.fig")),
    )
    .unwrap()
}
fn content(result: &CallToolResult) -> Vec<Value> {
    serde_json::to_value(result).unwrap()["content"]
        .as_array()
        .unwrap()
        .clone()
}
fn texts(result: &CallToolResult) -> Vec<String> {
    content(result)
        .iter()
        .filter(|c| c["type"] == "text")
        .map(|c| c["text"].as_str().unwrap().to_owned())
        .collect()
}
/// Decoded bytes and mime type of every image or blob item.
fn blobs(result: &CallToolResult) -> Vec<(Vec<u8>, String)> {
    content(result)
        .iter()
        .filter_map(|c| match c["type"].as_str()? {
            "image" => Some((c["data"].as_str()?, c["mimeType"].as_str()?)),
            "resource" => Some((
                c["resource"]["blob"].as_str()?,
                c["resource"]["mimeType"].as_str()?,
            )),
            _ => None,
        })
        .map(|(data, mime)| (STANDARD.decode(data).unwrap(), mime.to_owned()))
        .collect()
}
fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}
fn run(doc: &Document, nodes: &[&str], format: AssetFormat, scale: Option<f64>) -> CallToolResult {
    download(doc, &ids(nodes), format, scale, false, false)
}

#[test]
fn png_and_jpg_decode_to_the_node_size() {
    let doc = fixture("structure");
    for (format, mime, decoded) in [
        (AssetFormat::Png, "image/png", image::ImageFormat::Png),
        (AssetFormat::Jpg, "image/jpeg", image::ImageFormat::Jpeg),
    ] {
        let result = run(&doc, &["1:34"], format, None);
        assert_ne!(result.is_error, Some(true), "{:?}", texts(&result));
        let blobs = blobs(&result);
        assert_eq!(blobs.len(), 1);
        assert_eq!(blobs[0].1, mime);
        let img = image::load_from_memory_with_format(&blobs[0].0, decoded).unwrap();
        assert_eq!((img.width(), img.height()), (381, 191));
    }
}

#[test]
fn jpg_is_flattened_onto_white() {
    let doc = fixture("structure");
    // 1:24's corners are transparent; they must come out white, not black.
    let result = run(&doc, &["1:24"], AssetFormat::Jpg, Some(1.0));
    let jpg = image::load_from_memory(&blobs(&result)[0].0)
        .unwrap()
        .to_rgb8();
    let corner = jpg.get_pixel(0, 0);
    assert!(corner.0.iter().all(|c| *c > 240), "{corner:?}");
    assert!(texts(&result)[0].contains("flattened onto white"));
}

#[test]
fn several_nodes_include_instance_paths_and_a_bad_id_does_not_fail_the_call() {
    let doc = fixture("structure");
    let result = run(
        &doc,
        &["1:34", "I1:38;1:42", "9:99", "nonsense"],
        AssetFormat::Png,
        Some(0.5),
    );
    assert_ne!(result.is_error, Some(true));
    assert_eq!(blobs(&result).len(), 2);
    let lines = texts(&result);
    assert!(
        lines.iter().any(|l| l.starts_with("9:99: error:")),
        "{lines:?}"
    );
    assert!(lines.iter().any(|l| l.starts_with("nonsense: error:")));
}

#[test]
fn every_node_failing_is_a_tool_error() {
    let doc = fixture("structure");
    let result = run(&doc, &["9:99"], AssetFormat::Png, None);
    assert_eq!(result.is_error, Some(true));
}

#[test]
fn limits_are_enforced() {
    let doc = fixture("structure");
    let many: Vec<&str> = vec!["1:34"; 21];
    for bad in [
        run(&doc, &[], AssetFormat::Png, None),
        run(&doc, &many, AssetFormat::Png, None),
    ] {
        assert_eq!(bad.is_error, Some(true));
        assert!(texts(&bad)[0].contains("1 to 20"));
    }
    assert_eq!(
        run(&doc, &["1:34"], AssetFormat::Png, Some(4.5)).is_error,
        Some(true)
    );
    assert_eq!(
        run(&doc, &["1:34"], AssetFormat::Png, Some(0.0)).is_error,
        Some(true)
    );
    assert_ne!(
        run(&doc, &vec!["1:34"; 20], AssetFormat::Png, Some(0.1)).is_error,
        Some(true)
    );
    // The 4096 px cap comes from get_screenshot's renderer: a big scale stays within it.
    let big = run(&doc, &["0:1"], AssetFormat::Png, Some(4.0));
    let (bytes, _) = &blobs(&big)[0];
    let img = image::load_from_memory(bytes).unwrap();
    assert!(img.width().max(img.height()) <= 4096);
}

#[test]
fn originals_are_the_stored_bytes_and_report_their_type() {
    let doc = fixture("structure");
    // 1:24 and 1:34 each fill with one stored image.
    let result = download(
        &doc,
        &ids(&["1:24", "1:34"]),
        AssetFormat::Png,
        None,
        true,
        false,
    );
    assert_ne!(result.is_error, Some(true), "{:?}", texts(&result));
    let got = blobs(&result);
    assert_eq!(got.len(), 2);
    for (bytes, mime) in &got {
        assert!(doc.images.values().any(|stored| stored == bytes));
        let guessed = image::guess_format(bytes).unwrap().to_mime_type();
        assert_eq!(mime, guessed);
    }
    let want = [
        "9e5c0093c792b765075c6886d474cefd1c2d8809",
        "93e8eeb27e934c4b9ae9e7929c7df9e96a6ec90c",
    ];
    for (hash, (bytes, _)) in want.iter().zip(&got) {
        assert_eq!(&doc.images[*hash], bytes);
    }
}

#[test]
fn originals_report_missing_images_and_nodes_without_fills() {
    let doc = fixture("broken_images");
    // 1:3's image is not stored in the file, 1:4's is.
    let result = download(
        &doc,
        &ids(&["1:3", "1:4"]),
        AssetFormat::Png,
        None,
        true,
        false,
    );
    assert_eq!(blobs(&result).len(), 1);
    assert!(
        texts(&result)
            .iter()
            .any(|l| l.contains("1 image(s) not stored"))
    );
    let none = download(
        &fixture("structure"),
        &ids(&["1:26"]),
        AssetFormat::Png,
        None,
        true,
        false,
    );
    assert_eq!(none.is_error, Some(true));
    assert!(
        texts(&none)[0].contains("no image fills"),
        "{:?}",
        texts(&none)
    );
}

#[test]
fn originals_are_found_inside_frames() {
    let doc = fixture("structure");
    let all = download(&doc, &ids(&["0:1"]), AssetFormat::Png, None, true, false);
    // Every stored image the page's layers draw, each once.
    let hashes: std::collections::HashSet<_> = blobs(&all).into_iter().map(|b| b.0).collect();
    assert_eq!(hashes.len(), blobs(&all).len());
    assert!(blobs(&all).len() >= 2);
}

fn obj(fields: &[(&str, Val)]) -> Val {
    let mut out = Obj::new();
    for (key, value) in fields {
        out.insert(key, value.clone());
    }
    Val::Obj(out)
}
/// A sized node with an image fill of `hash` (when given) under `parent`.
fn layer(id: u32, parent: Guid, kind: &str, hash: Option<[u8; 20]>) -> Node {
    let mut node = Node::from_data(obj(&[
        ("guid", Guid(9, id).to_val()),
        ("parentIndex", obj(&[("guid", parent.to_val())])),
        ("type", Val::Str(kind.into())),
        ("name", Val::Str("synthetic".into())),
        (
            "size",
            obj(&[("x", Val::Float(10.0)), ("y", Val::Float(10.0))]),
        ),
    ]))
    .unwrap();
    if let Some(hash) = hash {
        let paint = obj(&[
            ("type", Val::Str("IMAGE".into())),
            ("image", obj(&[("hash", Val::Bytes(hash.to_vec()))])),
        ]);
        node.data.set("fillPaints", Val::Arr(vec![paint]));
    }
    node
}

#[test]
fn originals_are_capped_per_call_and_found_through_instances() {
    let mut doc = fixture("structure");
    let page = Guid(0, 1);
    let mut nodes = doc.nodes.clone();
    nodes.push(layer(1, page, "FRAME", None));
    for n in 0..21u8 {
        let hash = [n + 1; 20];
        doc.images.insert(hex(&hash), vec![n, 0xff]);
        nodes.push(layer(
            10 + u32::from(n),
            Guid(9, 1),
            "RECTANGLE",
            Some(hash),
        ));
    }
    // A component holding one image, and an instance of it.
    nodes.push(layer(50, page, "SYMBOL", None));
    nodes.push(layer(51, Guid(9, 50), "RECTANGLE", Some([99; 20])));
    doc.images
        .insert(hex(&[99; 20]), b"instance-bytes".to_vec());
    let mut instance = layer(52, page, "INSTANCE", None);
    instance
        .data
        .set("symbolData", obj(&[("symbolID", Guid(9, 50).to_val())]));
    nodes.push(instance);
    doc.set_nodes(nodes).unwrap();

    let capped = download(&doc, &ids(&["9:1"]), AssetFormat::Png, None, true, false);
    assert_eq!(blobs(&capped).len(), 20);
    assert!(
        texts(&capped).iter().any(|l| l.contains("1 omitted")),
        "{:?}",
        texts(&capped)
    );
    // Unknown bytes come back as an embedded blob with a generic type.
    assert!(
        blobs(&capped)
            .iter()
            .all(|b| b.1 == "application/octet-stream")
    );

    for id in ["9:52", "I9:52;9:51"] {
        let got = download(&doc, &ids(&[id]), AssetFormat::Png, None, true, false);
        assert_eq!(
            blobs(&got)[0].0,
            b"instance-bytes",
            "{id}: {:?}",
            texts(&got)
        );
    }
}

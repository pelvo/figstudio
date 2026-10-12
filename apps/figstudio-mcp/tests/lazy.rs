//! Lazily opened files: renders and original image bytes match an eager open, and a too-big
//! original is left unread.
use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp::{
    files::FileRegistry,
    tools::{
        assets::{AssetFormat, download, download_within},
        rendering::Rendering,
        screenshot::screenshot,
    },
};
use figstudio_render::images::Images;
use rmcp::model::CallToolResult;
use std::{path::PathBuf, sync::Arc};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig")
}
/// The fixture opened eagerly, and lazily through the registry, with its rendering.
async fn both() -> (Document, Arc<Document>, Rendering) {
    let eager = Document::open(fixture()).unwrap();
    let files = FileRegistry::new(vec![fixture()]).with_lazy_above(0);
    let lazy = files.open(None).await.unwrap();
    (eager, lazy, files.rendering(None, false).unwrap())
}
fn blobs(result: &CallToolResult) -> Vec<Vec<u8>> {
    let value = serde_json::to_value(result).unwrap();
    value["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["data"].as_str())
        .map(|data| STANDARD.decode(data).unwrap())
        .collect()
}
fn texts(result: &CallToolResult) -> String {
    let value = serde_json::to_value(result).unwrap();
    value["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn a_forced_lazy_registry_leaves_images_unread() {
    let (eager, lazy, _) = both().await;
    assert!(eager.lazy_images.is_empty());
    assert!(lazy.images.is_empty(), "lazy open read image bytes");
    let mut hashes: Vec<_> = lazy.lazy_images.keys().cloned().collect();
    hashes.sort();
    let mut stored: Vec<_> = eager.images.keys().cloned().collect();
    stored.sort();
    assert_eq!(hashes, stored);
    let default = FileRegistry::new(vec![fixture()]).open(None).await.unwrap();
    assert!(default.lazy_images.is_empty(), "a small file opens eagerly");
}

#[tokio::test]
async fn screenshots_of_image_fills_match_an_eager_open() {
    let (eager, lazy, rendering) = both().await;
    // 1:24 and 1:34 fill with images; 0:1 is the whole page.
    for (id, scale) in [("1:24", 1.0), ("1:34", 2.0), ("0:1", 0.25), ("0:1", 1.0)] {
        let want = screenshot(&eager, id, Some(scale), &Rendering::default());
        let got = screenshot(&lazy, id, Some(scale), &rendering);
        assert_eq!(texts(&want), texts(&got), "{id} at {scale}");
        assert!(!texts(&got).contains("images"), "{id}: {}", texts(&got));
        assert_eq!(blobs(&want), blobs(&got), "{id} at {scale}: pixels differ");
    }
}

#[tokio::test]
async fn original_bytes_match_an_eager_open() {
    let (eager, lazy, rendering) = both().await;
    let ids = vec!["0:1".to_owned()];
    let want = download(
        &eager,
        &ids,
        AssetFormat::Png,
        None,
        true,
        &Rendering::default(),
    );
    let got = download(&lazy, &ids, AssetFormat::Png, None, true, &rendering);
    assert!(blobs(&want).len() >= 2);
    assert_eq!(blobs(&want), blobs(&got));
    assert_eq!(texts(&want), texts(&got));
}

/// The budget is checked on the directory's size first: a file that cannot be read gives
/// "omitted" when the image is too big, "not stored" only when it fit and the read failed.
#[tokio::test]
async fn an_original_over_the_budget_is_never_read() {
    let (_, lazy, _) = both().await;
    let unreadable = Rendering::new(fixture().with_file_name("absent.fig"), Images::default());
    let ids = vec!["1:34".to_owned()];
    let tight = download_within(&lazy, &ids, AssetFormat::Png, None, true, &unreadable, 16);
    let text = texts(&tight);
    assert!(
        text.contains("0 image(s) not stored in the file, 1 omitted"),
        "{text}"
    );
    let roomy = download(&lazy, &ids, AssetFormat::Png, None, true, &unreadable);
    assert!(
        texts(&roomy).contains("1 image(s) not stored"),
        "{}",
        texts(&roomy)
    );
}

fn obj(fields: &[(&str, Val)]) -> Val {
    let mut out = Obj::new();
    for (key, value) in fields {
        out.insert(key, value.clone());
    }
    Val::Obj(out)
}
/// A component 9:2 holding a copy of the image-filled 1:24 (as 9:3), and an instance 9:10.
fn with_image_instance(doc: &Document) -> Document {
    let source = doc.node(Guid(1, 24)).unwrap();
    let size = source.data.get("size").unwrap().clone();
    let parent = |id: Guid| obj(&[("guid", id.to_val()), ("position", Val::Str("a".into()))]);
    let node = |id: u32, parent_id: Guid, kind: &str, extra: &[(&str, Val)]| {
        let mut data = obj(&[
            ("guid", Guid(9, id).to_val()),
            ("parentIndex", parent(parent_id)),
            ("type", Val::Str(kind.into())),
            ("size", size.clone()),
        ]);
        for (key, value) in extra {
            data.set(key, value.clone());
        }
        Node::from_data(data).unwrap()
    };
    let mut image = source.data.clone();
    image.set("guid", Guid(9, 3).to_val());
    image.set("parentIndex", parent(Guid(9, 2)));
    image.set(
        "transform",
        obj(&[("m00", Val::Float(1.0)), ("m11", Val::Float(1.0))]),
    );
    let symbol_data = obj(&[("symbolID", Guid(9, 2).to_val())]);
    let mut nodes = doc.nodes.clone();
    nodes.extend([
        node(2, Guid(0, 1), "SYMBOL", &[]),
        Node::from_data(image).unwrap(),
        node(10, Guid(0, 1), "INSTANCE", &[("symbolData", symbol_data)]),
    ]);
    let mut out = Document::from_bytes(&std::fs::read(fixture()).unwrap()).unwrap();
    out.lazy_images = doc.lazy_images.clone();
    out.images = doc.images.clone();
    out.set_nodes(nodes).unwrap();
    out
}

#[tokio::test]
async fn an_image_inside_an_instance_path_matches_an_eager_open() {
    let (eager, lazy, rendering) = both().await;
    let (eager, lazy) = (with_image_instance(&eager), with_image_instance(&lazy));
    assert!(lazy.images.is_empty() && !lazy.lazy_images.is_empty());
    let want = screenshot(&eager, "I9:10;9:3", Some(1.0), &Rendering::default());
    let got = screenshot(&lazy, "I9:10;9:3", Some(1.0), &rendering);
    assert_ne!(want.is_error, Some(true), "{}", texts(&want));
    assert!(!texts(&got).contains("images"), "{}", texts(&got));
    assert_eq!(blobs(&want), blobs(&got), "instance path pixels differ");
}

use super::*;

#[tokio::test]
async fn list_does_not_wait_for_a_pending_load() {
    let files = FileRegistry::new(vec![PathBuf::from("structure.fig")]);
    let cell = files.files[0].document.clone();
    let mut pending =
        Box::pin(cell.get_or_try_init(std::future::pending::<Result<Arc<Document>, McpError>>));
    assert!(futures_util::poll!(&mut pending).is_pending());
    let listed = tokio::time::timeout(std::time::Duration::from_millis(100), files.list())
        .await
        .expect("list blocked on loading");
    assert_eq!(listed[0].pages, None);
}

#[tokio::test]
async fn cancelled_open_still_populates_the_cache() {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let files = FileRegistry::new(vec![fixture]);
    let cell = files.files[0].document.clone();
    let mut blocked =
        Box::pin(cell.get_or_try_init(std::future::pending::<Result<Arc<Document>, McpError>>));
    assert!(futures_util::poll!(&mut blocked).is_pending());
    let mut open = Box::pin(files.open(None));
    assert!(futures_util::poll!(&mut open).is_pending());
    drop(open);
    drop(blocked);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while files.list().await[0].pages.is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled request discarded its load");
    let cached = cell.get().unwrap();
    assert!(Arc::ptr_eq(cached, &files.open(None).await.unwrap()));
}

#[test]
fn resolves_single_and_multiple_files() {
    let one = FileRegistry::new(vec![PathBuf::from("structure.fig")]);
    assert_eq!(one.resolve(None).unwrap().key, "structure");
    assert!(one.resolve(Some("unknown")).is_err());
    let two = FileRegistry::new(vec![
        PathBuf::from("structure.fig"),
        PathBuf::from("vector.fig"),
    ]);
    assert!(two.resolve(None).is_err());
    assert_eq!(
        two.resolve(Some("vector")).unwrap().path,
        PathBuf::from("vector.fig")
    );
    assert_eq!(two.resolve(Some("structure.fig")).unwrap().key, "structure");
    assert!(two.resolve(Some("unknown")).is_err());
    let absolute = std::path::absolute("vector.fig").unwrap();
    assert_eq!(two.resolve(absolute.to_str()).unwrap().key, "vector");
}

#[tokio::test]
async fn open_picked_uses_the_resolved_entry_for_duplicate_stems() {
    use figstudio_fig::Val;
    let dir = tempfile::tempdir().unwrap();
    let marker_path = |sub: &str, label: &str| {
        let mut doc = figstudio_fig::Document::open(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
        )
        .unwrap();
        let page = doc.pages[0];
        let mut nodes = doc.nodes.clone();
        let frame = nodes
            .iter_mut()
            .find(|n| n.kind == "FRAME" && n.parent == Some(page))
            .unwrap();
        frame.name = label.into();
        frame.data.set("name", Val::Str(label.into()));
        doc.set_nodes(nodes).unwrap();
        let sub = dir.path().join(sub);
        std::fs::create_dir_all(&sub).unwrap();
        let path = sub.join("dup.fig");
        doc.save(&path).unwrap();
        path
    };
    let older = marker_path("a", "picked-old-marker");
    let newer = marker_path("b", "picked-new-marker");
    let files = FileRegistry::new(vec![older.clone(), newer.clone()]);
    let (old_doc, old_key, old_path) = files
        .open_picked(Some(older.to_str().unwrap()))
        .await
        .unwrap();
    let (new_doc, new_key, new_path) = files
        .open_picked(Some(newer.to_str().unwrap()))
        .await
        .unwrap();
    assert_eq!((old_key.as_str(), new_key.as_str()), ("dup", "dup"));
    assert_eq!((old_path, new_path), (older, newer));
    // Each doc is its own file's: the renamed frame label proves identity.
    let label = |doc: &figstudio_fig::Document| {
        doc.nodes
            .iter()
            .find(|n| n.name.starts_with("picked-"))
            .unwrap()
            .name
            .clone()
    };
    assert_eq!(label(&old_doc), "picked-old-marker");
    assert_eq!(label(&new_doc), "picked-new-marker");
}

#[cfg(unix)]
#[test]
fn rendering_for_path_matches_non_utf8_registered_path() {
    use std::os::unix::ffi::OsStrExt;
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(b"bad\xff.fig"));
    let files = FileRegistry::new(vec![path.clone()]);
    assert!(files.rendering_for_path(&path, false).is_ok());
}

#[test]
fn duplicate_stems_require_paths() {
    let files = FileRegistry::new(vec![
        PathBuf::from("a/example.fig"),
        PathBuf::from("b/example.fig"),
    ]);
    assert!(files.resolve(Some("example")).is_err());
    assert_eq!(
        files.resolve(Some("b/example.fig")).unwrap().path,
        PathBuf::from("b/example.fig")
    );
    assert!(FileRegistry::new(vec![]).resolve(None).is_err());
}

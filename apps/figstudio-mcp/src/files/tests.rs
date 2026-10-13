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

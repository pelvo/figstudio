use figstudio_mcp::{files::FileRegistry, tools::tool_error};
use std::{path::PathBuf, sync::Arc};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig")
}

#[tokio::test]
async fn documents_load_lazily_and_share_the_cache() {
    let files = FileRegistry::new(vec![fixture()]);
    assert_eq!(files.list().await[0].pages, None);
    let document = files.open(None).await.unwrap();
    assert!(!document.pages.is_empty());
    assert_eq!(files.list().await[0].pages, Some(document.pages.len()));
    assert!(Arc::ptr_eq(
        &document,
        &files.open(Some("structure")).await.unwrap()
    ));
}

#[tokio::test]
async fn missing_and_bad_files_become_tool_errors() {
    // Public repository inputs only; the license is intentionally not a .fig.
    for path in [
        fixture().with_file_name("absent.fig"),
        fixture().with_file_name("LICENSE-fig2sketch"),
    ] {
        let files = FileRegistry::new(vec![path]);
        assert_eq!(files.list().await[0].pages, None);
        let error = match files.open(None).await {
            Err(error) => error,
            Ok(_) => panic!("bad file accepted"),
        };
        let result = tool_error(error);
        assert_eq!(result.is_error, Some(true));
        assert_eq!(files.list().await[0].pages, None);
    }
}

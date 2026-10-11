use figstudio_mcp::{files::FileRegistry, tools::tool_error};
use std::{path::PathBuf, sync::Arc};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig")
}

#[tokio::test]
async fn documents_load_lazily_and_share_the_cache() {
    let files = FileRegistry::new(vec![fixture()]);
    assert_eq!(files.list().await[0].pages, None);
    let (first, second) = tokio::join!(files.open(None), files.open(None));
    let document = first.unwrap();
    assert!(Arc::ptr_eq(&document, &second.unwrap()));
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
        let cause = std::error::Error::source(&error).unwrap().to_string();
        let result = tool_error(error);
        let text = serde_json::to_value(&result).unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(text.contains(&cause), "missing underlying error: {text}");
        assert_eq!(result.is_error, Some(true));
        assert_eq!(files.list().await[0].pages, None);
    }
}

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
async fn corrupt_zip_member_name_stays_out_of_open_errors() {
    // Regression pin: `Files::open` (via `open_picked`, the compare path)
    // wraps the decoder error, whose text names the corrupt member. That is
    // why compare maps `is_open` failures to fixed text (see tests/compare.rs).
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt.fig");
    let mut buf = vec![];
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("review-marker-member", options).unwrap();
        zip.write_all(b"not a canvas").unwrap();
        zip.start_file("canvas.fig", options).unwrap();
        zip.write_all(b"junk").unwrap();
        zip.finish().unwrap();
    }
    let at = buf.windows(12).position(|w| w == b"not a canvas").unwrap();
    buf[at] ^= 1;
    std::fs::write(&path, buf).unwrap();
    let files = FileRegistry::new(vec![path]);
    let error = files.open(None).await.unwrap_err();
    assert!(error.is_open(), "open failure must be classified: {error}");
    assert!(
        error.to_string().contains("review-marker-member"),
        "decoder detail expected at this layer: {error}"
    );
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

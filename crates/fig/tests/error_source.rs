//! `FigError` keeps the real cause so `source()` chains show it.

use std::error::Error;

use figstudio_fig::Document;

#[test]
fn opening_a_missing_file_keeps_the_io_error_as_source() {
    let dir = std::env::temp_dir().join(format!("figstudio-missing-{}", std::process::id()));
    let err = Document::open(dir.join("absent.fig")).unwrap_err();
    let source = err.source().expect("an I/O failure has a source");
    let io = source.downcast_ref::<std::io::Error>().expect("io::Error");
    assert_eq!(io.kind(), std::io::ErrorKind::NotFound);
    assert!(err.to_string().starts_with("cannot read file: "), "{err}");
}

#[test]
fn a_malformed_file_has_no_source() {
    let err = Document::from_bytes(b"not a figma file at all").unwrap_err();
    assert!(err.source().is_none(), "{err}");
}

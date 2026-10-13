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

#[test]
fn one_line_names_each_cause_once() {
    let dir = std::env::temp_dir().join(format!("figstudio-one-line-{}", std::process::id()));
    let err = Document::open(dir.join("absent.fig")).unwrap_err();
    let cause = err.source().unwrap().to_string();
    let line = figstudio_fig::one_line(&err);
    assert!(line.starts_with("cannot read file: "), "{line}");
    assert_eq!(line.matches(cause.as_str()).count(), 1, "{line}");

    // A wrapper that does not repeat its cause gets it appended.
    #[derive(Debug)]
    struct Outer(std::io::Error);
    impl std::fmt::Display for Outer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("outer")
        }
    }
    impl Error for Outer {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.0)
        }
    }
    let outer = Outer(std::io::Error::other("root"));
    assert_eq!(figstudio_fig::one_line(&outer), "outer: root");
}

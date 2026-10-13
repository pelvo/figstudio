//! Render errors keep their underlying cause reachable through `source()`.
use figstudio_fig::{Document, resolve::ResolveError};
use figstudio_mcp_core::{rendering::Rendering, screenshot::render_node};
use std::{error::Error, path::PathBuf};

#[test]
fn an_unresolvable_instance_path_keeps_the_resolve_error_as_its_source() {
    let doc = Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
    )
    .unwrap();
    let Err(error) = render_node(&doc, "I99:99;1:2", None, &Rendering::default()) else {
        panic!("a path through a missing instance must not render");
    };
    assert!(error.to_string().starts_with("I99:99;1:2: "), "{error}");
    let cause = error.source().expect("the resolve error is the source");
    assert!(cause.is::<ResolveError>());
    assert_eq!(error.to_string(), format!("I99:99;1:2: {cause}"));
}

#[test]
fn an_encode_error_keeps_its_cause_and_the_old_text() {
    let cause = std::io::Error::other("disk full");
    let error = figstudio_mcp_core::CoreError::encode("PNG", cause);
    assert_eq!(error.to_string(), "PNG encoding failed: disk full");
    let source = error.source().expect("the encoder error is the source");
    assert_eq!(source.to_string(), "disk full");
}

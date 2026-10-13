//! `Output` constructors and the core text tools on a public fixture.

use figstudio_fig::Document;
use figstudio_mcp_core::{
    metadata::{DEFAULT_OUTPUT_BYTES, metadata_within},
    output::{Block, Output, text_first},
    variables::variable_defs,
};
use serde_json::json;

const STRUCTURE: &[u8] = include_bytes!("../../../tests/fixtures/structure.fig");

#[test]
fn structured_carries_its_json_text_and_value() {
    let out = Output::structured(json!({"a": 1}));
    assert!(!out.is_error);
    assert_eq!(out.structured, Some(json!({"a": 1})));
    assert_eq!(out.content, vec![Block::text(r#"{"a":1}"#)]);
}

#[test]
fn fault_is_an_error_with_one_text_block() {
    let out = Output::fault("nope");
    assert!(out.is_error && out.structured.is_none());
    assert_eq!(out.content, vec![Block::text("nope")]);
}

#[test]
fn text_first_keeps_order_within_each_kind() {
    let img = Block::image("AA", "image/png");
    let got = text_first(vec![
        Block::text("1"),
        img.clone(),
        Block::text("2"),
        img.clone(),
    ]);
    assert_eq!(
        got,
        vec![Block::text("1"), Block::text("2"), img.clone(), img]
    );
}

#[test]
fn text_tools_answer_without_a_server() {
    let doc = Document::from_bytes(STRUCTURE).expect("fixture opens");
    let pages = metadata_within(&doc, None, None, DEFAULT_OUTPUT_BYTES);
    assert!(!pages.is_error);
    assert!(
        pages.content[0]
            .as_text()
            .is_some_and(|t| t.starts_with("<pages"))
    );
    let bad = metadata_within(&doc, Some("not an id"), None, DEFAULT_OUTPUT_BYTES);
    assert!(bad.is_error);
    assert!(variable_defs(&doc, "bogus").is_error);
}

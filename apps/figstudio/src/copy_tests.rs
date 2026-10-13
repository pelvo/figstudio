//! Copy formatters, on `structure.fig` and on a document built in the test.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp_core::link::parse_link;
use figstudio_mcp_core::node_id::{NodeId, parse_node_id};

use super::*;

const STRUCTURE: &[u8] = include_bytes!("../../../tests/fixtures/structure.fig");
/// The TEXT node "ABC" inside the SYMBOL "Component 1" on "Page 1".
const TEXT_NODE: Guid = Guid(1, 42);
/// The SYMBOL "Component 1", which has no text of its own.
const SYMBOL_NODE: Guid = Guid(1, 37);

fn tab_for(path: &str, doc: Document) -> Tab {
    Tab::new(1, PathBuf::from(path), Arc::new(doc), None, Duration::ZERO)
}

fn structure() -> Tab {
    tab_for(
        "/private/dir/structure.fig",
        Document::from_bytes(STRUCTURE).unwrap(),
    )
}

fn obj(fields: &[(&str, Val)]) -> Val {
    let mut o = Obj::new();
    for (k, v) in fields {
        o.insert(k, v.clone());
    }
    Val::Obj(o)
}

fn guid(n: u32) -> Val {
    Guid(9, n).to_val()
}

/// A TEXT node `9:2` with the given characters, on canvas `9:1`.
fn text_node(characters: &str) -> Node {
    let parent = obj(&[("guid", guid(1)), ("position", Val::Str("000".into()))]);
    Node::from_data(obj(&[
        ("guid", guid(2)),
        ("type", Val::Str("TEXT".into())),
        ("name", Val::Str("Caption".into())),
        ("parentIndex", parent),
        (
            "textData",
            obj(&[("characters", Val::Str(characters.into()))]),
        ),
    ]))
    .unwrap()
}

fn doc_with(nodes: Vec<Node>) -> Document {
    let canvas = Node::from_data(obj(&[
        ("guid", guid(1)),
        ("type", Val::Str("CANVAS".into())),
        ("name", Val::Str("Page 1".into())),
        (
            "parentIndex",
            obj(&[("guid", guid(0)), ("position", Val::Str("0".into()))]),
        ),
    ]))
    .unwrap();
    let mut doc = Document::from_bytes(STRUCTURE).unwrap();
    let mut all = vec![canvas];
    all.extend(nodes);
    doc.set_nodes(all).unwrap();
    doc.pages = vec![Guid(9, 1)];
    doc
}

fn kinds(items: &[Item]) -> Vec<&'static str> {
    items.iter().map(|i| i.label).collect()
}

#[test]
fn name_is_the_stored_name() {
    let tab = structure();
    let node = tab.doc.node(SYMBOL_NODE).unwrap();
    assert_eq!(node.name, "Component 1");
}

#[test]
fn text_is_the_characters_of_a_text_node_only() {
    let tab = structure();
    assert_eq!(text(tab.doc.node(TEXT_NODE).unwrap()), Some("ABC"));
    assert_eq!(text(tab.doc.node(SYMBOL_NODE).unwrap()), None);
}

#[test]
fn text_keeps_more_than_the_properties_preview() {
    let long: String = "word ".repeat(400);
    let tab = tab_for("x.fig", doc_with(vec![text_node(&long)]));
    let got = text(tab.doc.node(Guid(9, 2)).unwrap()).unwrap();
    assert_eq!(got.chars().count(), 2000);
    assert_eq!(got, long);
}

#[test]
fn node_id_is_colon_form() {
    assert_eq!(TEXT_NODE.to_string(), "1:42");
}

#[test]
fn file_key_is_the_stem_not_the_path() {
    let tab = structure();
    assert_eq!(file_key(&tab), "structure");
    assert!(!file_key(&tab).contains('/'));
}

#[test]
fn file_key_falls_back_to_the_document_name() {
    let mut doc = Document::from_bytes(STRUCTURE).unwrap();
    doc.file_name = Some("Sales deck.fig".into());
    let tab = tab_for("", doc);
    assert_eq!(file_key(&tab), "Sales deck");
}

#[test]
fn link_round_trips_to_the_same_file_and_node() {
    let tab = structure();
    let url = link(&file_key(&tab), TEXT_NODE);
    assert_eq!(url, "figstudio://structure?node-id=1-42");
    let back = parse_link(&url).unwrap();
    assert_eq!(back.node, Some(NodeId::Node(TEXT_NODE)));
    assert_eq!(parse_node_id(&url).unwrap(), NodeId::Node(TEXT_NODE));
}

#[test]
fn agent_block_has_three_lines_with_the_box() {
    let tab = structure();
    let node = tab.doc.node(SYMBOL_NODE).unwrap();
    let block = agent_block(&tab, node);
    let lines: Vec<&str> = block.lines().collect();
    assert_eq!(lines.len(), 3, "{block}");
    assert_eq!(lines[0], "figstudio://structure?node-id=1-37");
    assert_eq!(lines[1], "file: structure.fig · page: Page 1");
    assert!(
        lines[2].starts_with("node: 1:37 \"Component 1\" (SYMBOL) x="),
        "{}",
        lines[2]
    );
    assert!(lines[2].contains(" w=") && lines[2].contains(" h="));
}

#[test]
fn agent_block_flattens_a_multi_line_name() {
    let tab = tab_for(
        "x.fig",
        doc_with(vec![
            Node::from_data(obj(&[
                ("guid", guid(2)),
                ("type", Val::Str("FRAME".into())),
                ("name", Val::Str("Row\nTwo".into())),
                (
                    "parentIndex",
                    obj(&[("guid", guid(1)), ("position", Val::Str("0".into()))]),
                ),
            ]))
            .unwrap(),
        ]),
    );
    let block = agent_block(&tab, tab.doc.node(Guid(9, 2)).unwrap());
    assert_eq!(block.lines().count(), 3, "{block}");
    assert!(block.contains("\"Row Two\""));
}

#[test]
fn items_for_a_text_node_list_all_five() {
    let tab = structure();
    let items = items(&tab, TEXT_NODE);
    assert_eq!(
        kinds(&items),
        [
            "Copy name",
            "Copy text",
            "Copy node ID",
            "Copy link",
            "Copy for agent"
        ]
    );
    assert_eq!(items[1].text, "ABC");
    assert_eq!(items[2].text, "1:42");
    assert_eq!(items[3].text, "figstudio://structure?node-id=1-42");
}

#[test]
fn items_leave_out_text_for_a_non_text_node() {
    let tab = structure();
    let items = items(&tab, SYMBOL_NODE);
    assert!(!kinds(&items).contains(&"Copy text"));
    assert_eq!(items.len(), 4);
}

#[test]
fn items_are_empty_for_an_unknown_node() {
    let tab = structure();
    assert!(items(&tab, Guid(7, 7)).is_empty());
}

#[test]
fn num_drops_trailing_zeros() {
    assert_eq!(num(120.0), "120");
    assert_eq!(num(33.5), "33.5");
    assert_eq!(num(1.234), "1.23");
}

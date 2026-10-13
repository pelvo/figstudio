//! `region::nodes_in` on documents built here, over the structure fixture's schema.

use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp_core::region::nodes_in;

const STRUCTURE: &[u8] = include_bytes!("../../../tests/fixtures/structure.fig");
const PAGE: Guid = Guid(9, 1);

fn obj(fields: Vec<(&str, Val)>) -> Val {
    let mut o = Obj::new();
    for (k, v) in fields {
        o.insert(k, v);
    }
    Val::Obj(o)
}

fn num(v: f64) -> Val {
    Val::Float(v)
}

/// A frame `9:n` named `name` at (x, y), w × h, child of `parent`; `visible=false` hides it.
fn frame(n: u32, name: &str, parent: u32, at: (f64, f64, f64, f64), visible: bool) -> Node {
    let mut fields = vec![
        ("guid", Guid(9, n).to_val()),
        ("type", Val::Str("FRAME".into())),
        ("name", Val::Str(name.into())),
        (
            "parentIndex",
            obj(vec![
                ("guid", Guid(9, parent).to_val()),
                ("position", Val::Str(format!("{n:03}"))),
            ]),
        ),
        ("size", obj(vec![("x", num(at.2)), ("y", num(at.3))])),
        (
            "transform",
            obj(vec![("m02", num(at.0)), ("m12", num(at.1))]),
        ),
    ];
    if !visible {
        fields.push(("visible", Val::Bool(false)));
    }
    Node::from_data(obj(fields)).unwrap()
}

fn doc_with(nodes: Vec<Node>) -> Document {
    let canvas = Node::from_data(obj(vec![
        ("guid", PAGE.to_val()),
        ("type", Val::Str("CANVAS".into())),
        ("name", Val::Str("Page 1".into())),
        (
            "parentIndex",
            obj(vec![
                ("guid", Guid(9, 0).to_val()),
                ("position", Val::Str("0".into())),
            ]),
        ),
    ]))
    .unwrap();
    let mut doc = Document::from_bytes(STRUCTURE).unwrap();
    let mut all = vec![canvas];
    all.extend(nodes);
    doc.set_nodes(all).unwrap();
    doc.pages = vec![PAGE];
    doc
}

const ALL: [f64; 4] = [-1000.0, -1000.0, 5000.0, 5000.0];

fn ids(doc: &Document, rect: [f64; 4], limit: usize, budget: usize) -> Vec<u32> {
    nodes_in(doc, PAGE, rect, limit, budget)
        .nodes
        .iter()
        .map(|n| n.id.1)
        .collect()
}

#[test]
fn top_level_nodes_come_before_their_children() {
    // 2 is a top-level frame holding child 4; 3 is another top-level frame. Depth-first would
    // give 2, 4, 3.
    let doc = doc_with(vec![
        frame(2, "A", 1, (0.0, 0.0, 100.0, 100.0), true),
        frame(3, "B", 1, (200.0, 0.0, 100.0, 100.0), true),
        frame(4, "A child", 2, (10.0, 10.0, 20.0, 20.0), true),
    ]);
    assert_eq!(ids(&doc, ALL, 50, 1000), [2, 3, 4]);
}

#[test]
fn only_nodes_touching_the_rect_are_listed() {
    let doc = doc_with(vec![
        frame(2, "A", 1, (0.0, 0.0, 100.0, 100.0), true),
        frame(3, "B", 1, (200.0, 0.0, 100.0, 100.0), true),
    ]);
    assert_eq!(ids(&doc, [50.0, 50.0, 10.0, 10.0], 50, 1000), [2]);
    // Edges that only touch do not count.
    assert_eq!(
        ids(&doc, [100.0, 0.0, 50.0, 50.0], 50, 1000),
        Vec::<u32>::new()
    );
}

#[test]
fn invisible_nodes_and_their_children_are_skipped() {
    let doc = doc_with(vec![
        frame(2, "Hidden", 1, (0.0, 0.0, 100.0, 100.0), false),
        frame(3, "Inside hidden", 2, (0.0, 0.0, 10.0, 10.0), true),
        frame(4, "Shown", 1, (0.0, 0.0, 100.0, 100.0), true),
    ]);
    assert_eq!(ids(&doc, ALL, 50, 1000), [4]);
}

#[test]
fn the_limit_caps_the_list_but_not_the_total() {
    let doc = doc_with(
        (2..8)
            .map(|n| frame(n, "N", 1, (0.0, 0.0, 10.0, 10.0), true))
            .collect(),
    );
    let region = nodes_in(&doc, PAGE, ALL, 4, 1000);
    assert_eq!(region.nodes.len(), 4);
    assert_eq!(region.total, 6);
    assert!(region.truncated && !region.exhausted);
    let all = nodes_in(&doc, PAGE, ALL, 50, 1000);
    assert!(!all.truncated);
}

#[test]
fn a_spent_budget_stops_the_walk_and_says_so() {
    let doc = doc_with(
        (2..8)
            .map(|n| frame(n, "N", 1, (0.0, 0.0, 10.0, 10.0), true))
            .collect(),
    );
    let region = nodes_in(&doc, PAGE, ALL, 50, 3);
    assert_eq!(region.nodes.len(), 3);
    assert!(region.exhausted && region.truncated);
}

#[test]
fn a_missing_page_or_empty_rect_gives_nothing() {
    let doc = doc_with(vec![frame(2, "A", 1, (0.0, 0.0, 100.0, 100.0), true)]);
    assert!(nodes_in(&doc, Guid(7, 7), ALL, 50, 1000).nodes.is_empty());
    assert!(
        nodes_in(&doc, PAGE, [10.0, 10.0, 0.0, 0.0], 50, 1000)
            .nodes
            .is_empty()
    );
}

#[test]
fn names_and_kinds_are_carried() {
    let doc = doc_with(vec![frame(
        2,
        "Button / Primary",
        1,
        (0.0, 0.0, 9.0, 9.0),
        true,
    )]);
    let region = nodes_in(&doc, PAGE, ALL, 50, 1000);
    assert_eq!(region.nodes[0].name, "Button / Primary");
    assert_eq!(region.nodes[0].kind, "FRAME");
}

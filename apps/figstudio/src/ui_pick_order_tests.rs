//! Paint-order and bounded-work regressions, using synthetic public-schema documents.
use super::inner_fixture::{id, path};
use super::*;
use egui::{Key, Modifiers};
use figstudio_fig::{Document, Node, Val};
use figstudio_render::kurbo::Point;
use std::sync::Arc;

fn child(doc: &Document, n: u32, parent: u32, kind: &str) -> Node {
    let mut node = doc.node(id(105)).unwrap().clone();
    node.id = id(n);
    node.kind = kind.into();
    node.data.set("guid", id(n).to_val());
    node.data.set("type", Val::Str(kind.into()));
    let index = node.data.get_mut("parentIndex").unwrap();
    index.set("guid", id(parent).to_val());
    index.set("position", Val::Str(format!("{n:06}")));
    let tf = node.data.get_mut("transform").unwrap();
    tf.set("m02", Val::Float(0.0));
    tf.set("m12", Val::Float(0.0));
    Node::from_data(node.data).unwrap()
}

fn ordered(reverse: bool) -> Document {
    let mut doc = inner_fixture::document();
    let mut nodes = doc.nodes.clone();
    nodes.retain(|n| !matches!(n.id, Guid(99, 102..=107)));
    nodes.extend((300..=302).map(|n| child(&doc, n, 101, "RECTANGLE")));
    let excluded = [
        (299, "SECTION"),
        (303, "SECTION"),
        (298, "RECTANGLE"),
        (304, "RECTANGLE"),
        (297, "RECTANGLE"),
        (305, "RECTANGLE"),
    ];
    for (n, kind) in excluded {
        let mut node = child(&doc, n, 101, kind);
        if matches!(n, 298 | 304) {
            node.data.set("visible", Val::Bool(false));
        }
        if matches!(n, 297 | 305) {
            node.data.set("mask", Val::Bool(true));
        }
        nodes.push(node);
    }
    nodes
        .iter_mut()
        .find(|n| n.id == id(101))
        .unwrap()
        .data
        .set("stackReverseZIndex", Val::Bool(reverse));
    doc.set_nodes(nodes).unwrap();
    doc
}

#[test]
fn enter_picks_topmost_visible_child_and_skips_sections() {
    for reverse in [true, false] {
        let mut h = inner_fixture::harness();
        let mut doc = ordered(reverse);
        let mut nodes = doc.nodes.clone();
        let group = nodes.iter_mut().find(|n| n.id == id(101)).unwrap();
        group.parent = Some(id(2));
        group
            .data
            .get_mut("parentIndex")
            .unwrap()
            .set("guid", id(2).to_val());
        doc.set_nodes(nodes).unwrap();
        h.state_mut().tabs[0].doc = Arc::new(doc);
        h.state_mut().tabs[0].select(Some(id(101)));
        text_select::key(&mut h, &mut Vec::new(), Key::Enter, Modifiers::NONE);
        assert_eq!(
            h.state().tabs[0].selection,
            Some(id(if reverse { 300 } else { 302 }))
        );
    }
}

#[test]
fn enter_in_instance_uses_resolved_stacking_order() {
    for reverse in [true, false] {
        let mut h = inner_fixture::harness();
        let mut doc = ordered(reverse);
        // Move the ordered children directly into the symbol so its order, not the
        // instance's contrary flag, determines the first resolved child.
        let mut nodes = doc.nodes.clone();
        nodes.retain(|n| n.id != id(101));
        for n in &mut nodes {
            if n.parent == Some(id(101)) {
                n.parent = Some(id(100));
                n.data
                    .get_mut("parentIndex")
                    .unwrap()
                    .set("guid", id(100).to_val());
            }
            if n.id == id(100) || n.id == id(200) {
                n.data.set(
                    "stackReverseZIndex",
                    Val::Bool(if n.id == id(100) { reverse } else { !reverse }),
                );
            }
        }
        doc.set_nodes(nodes).unwrap();
        h.state_mut().tabs[0].doc = Arc::new(doc);
        h.state_mut().tabs[0].select(Some(id(200)));
        text_select::key(&mut h, &mut Vec::new(), Key::Enter, Modifiers::NONE);
        assert_eq!(
            h.state().tabs[0].inner,
            Some(path(&[if reverse { 300 } else { 302 }]))
        );
    }
}

fn wide(reverse: bool, hidden: bool) -> (Document, u32) {
    let mut doc = inner_fixture::document();
    let mut nodes = doc.nodes.clone();
    nodes.retain(|n| !matches!(n.id, Guid(99, 101..=107)));
    let top = if reverse { 300 } else { 2399 };
    for n in 300..2400 {
        let mut node = child(&doc, n, 100, if n == top { "GROUP" } else { "RECTANGLE" });
        if hidden && n != top {
            node.data.set("visible", Val::Bool(false));
        }
        nodes.push(node);
    }
    nodes.push(child(&doc, 2500, top, "RECTANGLE"));
    nodes
        .iter_mut()
        .find(|n| n.id == id(100))
        .unwrap()
        .data
        .set("stackReverseZIndex", Val::Bool(reverse));
    doc.set_nodes(nodes).unwrap();
    (doc, top)
}

#[test]
fn wide_instance_keeps_frontmost_child_and_descends_with_remaining_budget() {
    for reverse in [true, false] {
        let (doc, top) = wide(reverse, false);
        assert_eq!(
            crate::instance_pick::hit_chain(&doc, id(200), Point::new(5.0, 5.0)),
            vec![path(&[top]), path(&[2500])]
        );
    }
}

#[test]
fn wide_instance_hidden_prefix_cannot_hide_frontmost_child() {
    let (doc, top) = wide(false, true);
    assert_eq!(
        crate::instance_pick::hit_chain(&doc, id(200), Point::new(5.0, 5.0)),
        vec![path(&[top]), path(&[2500])]
    );
}

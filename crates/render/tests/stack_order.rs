//! Reverse z-index changes painting order, not the serialized child order.
mod common;
use common::*;
use figstudio_fig::Val;
use figstudio_render::{hit_test, kurbo::Point};

#[test]
fn reverse_z_index_keeps_first_instance_above_later_overlapping_siblings() {
    let mut front = node(3, 2, "RECTANGLE");
    front.data.set("fillPaints", paint(0.0, 0.0, 1.0));
    let mut back = node(6, 5, "RECTANGLE");
    back.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    let mut parent = node(8, 1, "FRAME");
    parent.data.set("stackReverseZIndex", Val::Bool(true));
    let mut doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        front,
        node(5, 0, "SYMBOL"),
        back,
        parent,
        instance(9, 8, 2, vec![]),
        instance(10, 8, 5, vec![]),
    ]);
    assert_eq!(pixel(&pixels(&doc), 10, 10), &[0, 0, 255, 255]);
    let mut nodes = doc.nodes.clone();
    nodes
        .iter_mut()
        .find(|n| n.id == id(8))
        .unwrap()
        .data
        .set("stackReverseZIndex", Val::Bool(false));
    doc.set_nodes(nodes).unwrap();
    assert_eq!(pixel(&pixels(&doc), 10, 10), &[255, 0, 0, 255]);
}

#[test]
fn hit_test_selects_the_layer_painted_on_top_under_reverse_z_index() {
    let mut front = node(3, 2, "RECTANGLE");
    front.data.set("fillPaints", paint(0.0, 0.0, 1.0));
    let mut back = node(6, 5, "RECTANGLE");
    back.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    let mut parent = node(8, 1, "FRAME");
    parent.data.set("stackReverseZIndex", Val::Bool(true));
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        front,
        node(5, 0, "SYMBOL"),
        back,
        parent,
        instance(9, 8, 2, vec![]),
        instance(10, 8, 5, vec![]),
    ]);
    assert_eq!(
        pixel(&pixels(&doc), 10, 10),
        &[0, 0, 255, 255],
        "9 is painted on top"
    );
    assert_eq!(hit_test(&doc, id(1), Point::new(10.0, 10.0)), Some(id(9)));
}

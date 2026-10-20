mod common;

use common::*;
use figstudio_fig::{Document, Guid, Val};
use figstudio_render::{hit_chain, hit_test, kurbo::Point};

#[test]
fn chain_preserves_ancestors_and_deepest_contract() {
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 1, "FRAME"),
        node(3, 2, "GROUP"),
        node(4, 3, "TEXT"),
    ]);
    let point = Point::new(10.0, 10.0);
    assert_eq!(hit_chain(&doc, id(1), point), [id(2), id(3), id(4)]);
    assert_eq!(hit_test(&doc, id(1), point), Some(id(4)));
    assert!(hit_chain(&doc, id(1), Point::new(100.0, 100.0)).is_empty());
}

#[test]
fn sections_are_not_selectable_but_instances_are_leaves() {
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 1, "SECTION"),
        node(3, 2, "INSTANCE"),
        node(4, 3, "TEXT"),
    ]);
    assert_eq!(hit_chain(&doc, id(1), Point::new(10.0, 10.0)), [id(3)]);
    let doc = document(vec![node(1, 0, "CANVAS"), node(2, 1, "SECTION")]);
    assert!(hit_chain(&doc, id(1), Point::new(10.0, 10.0)).is_empty());
}

#[test]
fn hit_order_visibility_and_masks_match_painting() {
    let mut hidden = node(5, 2, "RECTANGLE");
    hidden.data.set("visible", Val::Bool(false));
    let mut mask = node(6, 2, "RECTANGLE");
    mask.data.set("mask", Val::Bool(true));
    let mut nodes = vec![
        node(1, 0, "CANVAS"),
        node(2, 1, "GROUP"),
        node(3, 2, "RECTANGLE"),
        node(4, 2, "RECTANGLE"),
        hidden,
        mask,
    ];
    let point = Point::new(10.0, 10.0);
    assert_eq!(
        hit_chain(&document(nodes.clone()), id(1), point),
        [id(2), id(4)]
    );
    nodes[1].data.set("stackReverseZIndex", Val::Bool(true));
    assert_eq!(hit_chain(&document(nodes), id(1), point), [id(2), id(3)]);
}

#[test]
fn clip_and_overflow_use_the_same_transformed_walk() {
    let mut frame = node(2, 1, "FRAME");
    let mut child = node(3, 2, "TEXT");
    child.data.set(
        "transform",
        obj(&[
            ("m00", Val::Float(1.0)),
            ("m11", Val::Float(1.0)),
            ("m02", Val::Float(50.0)),
        ]),
    );
    let point = Point::new(60.0, 10.0);
    let doc = document(vec![node(1, 0, "CANVAS"), frame.clone(), child.clone()]);
    assert_eq!(hit_chain(&doc, id(1), point), [id(2), id(3)]);
    frame.data.set("frameMaskDisabled", Val::Bool(false));
    let doc = document(vec![node(1, 0, "CANVAS"), frame, child]);
    assert!(hit_chain(&doc, id(1), point).is_empty());
}

#[test]
fn depth_is_bounded_on_hostile_nesting() {
    let mut nodes = vec![node(1, 0, "CANVAS")];
    nodes.extend((2..200).map(|i| node(i, i - 1, "GROUP")));
    let chain = hit_chain(&document(nodes), id(1), Point::new(10.0, 10.0));
    assert_eq!(chain.len(), 128);
}

#[test]
fn structure_chain_stops_at_the_instance() {
    let doc = Document::open(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/structure.fig"),
    )
    .unwrap();
    let instance = Guid(1, 38);
    let bounds = figstudio_render::node_bounds(&doc, instance).unwrap();
    let chain = hit_chain(&doc, doc.page_of(instance).unwrap(), bounds.center());
    assert_eq!(chain, [Guid(1, 4), instance]);
    assert_eq!(
        hit_test(&doc, doc.page_of(instance).unwrap(), bounds.center()),
        Some(instance)
    );
}

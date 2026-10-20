use super::*;
use egui::Modifiers;
use figstudio_fig::{Node, Val};
use std::sync::Arc;

#[test]
fn cousin_click_stays_at_the_nearest_common_ancestor() {
    let mut h = pick::nested_harness();
    let mut doc = pick::nested();
    let mut group = doc.node(pick::NEST).unwrap().data.clone();
    group.set("guid", Guid(99, 9).to_val());
    let mut tf = group.get("transform").unwrap().clone();
    tf.set("m02", Val::Float(100.0));
    group.set("transform", tf);
    let mut leaf = doc.node(pick::TEXT).unwrap().data.clone();
    leaf.set("guid", Guid(99, 10).to_val());
    let mut parent = leaf.get("parentIndex").unwrap().clone();
    parent.set("guid", Guid(99, 9).to_val());
    leaf.set("parentIndex", parent);
    let mut nodes = doc.nodes.clone();
    nodes.retain(|n| n.id != pick::SIBLING);
    nodes.extend([
        Node::from_data(group).unwrap(),
        Node::from_data(leaf).unwrap(),
    ]);
    doc.set_nodes(nodes).unwrap();
    h.state_mut().tabs[0].doc = Arc::new(doc);
    h.run_steps(3);
    let at = centre_of(&h, pick::TEXT);
    let mut c = Vec::new();
    pick::click(&mut h, at, Modifiers::NONE);
    text_select::double_click(&mut h, &mut c, at);
    text_select::double_click(&mut h, &mut c, at);
    assert_eq!(h.state().tabs[0].selection, Some(pick::TEXT));
    let at = centre_of(&h, Guid(99, 10));
    pick::click(&mut h, at, Modifiers::NONE);
    // Both groups are inside a wrapper: resetting to the top-frame default is too shallow.
    assert_eq!(h.state().tabs[0].selection, Some(Guid(99, 9)));
    assert!(h.state().tabs[0].text_select.is_none());
}

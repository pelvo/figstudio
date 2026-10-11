//! The public instance resolver (`fig::resolve`) on documents built in the test.
mod common;
use common::*;
use figstudio_fig::resolve::{InstancePath, resolve_instance, resolve_path};
use figstudio_fig::{Document, Val};

fn text(chars: &str) -> Val {
    obj(&[("characters", Val::Str(chars.into()))])
}

fn path(ids: &[u32]) -> InstancePath {
    InstancePath::new(ids.iter().map(|n| id(*n)).collect()).unwrap()
}

/// The resolved data at `ids`, checked to be the same through the walk and the lookup.
fn at(doc: &Document, ids: &[u32]) -> Val {
    let want = path(ids);
    let walk = resolve_instance(doc, want.instance()).unwrap();
    let walked = walk.nodes.iter().find(|r| r.path == want).unwrap();
    let looked_up = resolve_path(doc, &want).unwrap();
    assert_eq!(
        walked.data, looked_up.data,
        "walk and lookup disagree at {want}"
    );
    looked_up.data
}

#[test]
fn text_and_fill_overrides_apply_to_the_child() {
    let mut label = node(3, 2, "TEXT");
    label.data.set("textData", text("old"));
    let mut rect = node(4, 2, "RECTANGLE");
    rect.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        label,
        rect,
        instance(
            10,
            1,
            2,
            vec![
                patch(&[3], &[("textData", text("new"))]),
                patch(&[4], &[("fillPaints", paint(0.0, 1.0, 0.0))]),
            ],
        ),
    ]);
    let label = at(&doc, &[10, 3]);
    assert_eq!(
        label.path(&["textData", "characters"]),
        Some(&Val::Str("new".into()))
    );
    assert_eq!(
        at(&doc, &[10, 4]).get("fillPaints"),
        Some(&paint(0.0, 1.0, 0.0))
    );
    assert_eq!(
        pixel(&pixels(&doc), 20, 20),
        &[0, 255, 0, 255],
        "renderer agrees"
    );
    let walk = resolve_instance(&doc, id(10)).unwrap();
    let ids: Vec<String> = walk.nodes.iter().map(|r| r.path.to_string()).collect();
    assert_eq!(ids, ["I99:10;99:3", "I99:10;99:4"]);
}

#[test]
fn a_component_property_hides_a_child() {
    let mut rect = node(3, 2, "RECTANGLE");
    rect.data.set(
        "componentPropRefs",
        Val::Arr(vec![obj(&[
            ("defID", id(90).to_val()),
            ("componentPropNodeField", Val::Str("VISIBLE".into())),
        ])]),
    );
    let mut top = instance(10, 1, 2, vec![]);
    top.data.set(
        "componentPropAssignments",
        Val::Arr(vec![obj(&[
            ("defID", id(90).to_val()),
            ("value", obj(&[("boolValue", Val::Bool(false))])),
        ])]),
    );
    let doc = document(vec![node(1, 0, "CANVAS"), node(2, 0, "SYMBOL"), rect, top]);
    assert_eq!(at(&doc, &[10, 3]).flag("visible"), Some(false));
}

#[test]
fn a_swapped_nested_instance_lists_the_new_component_children() {
    let mut old = node(5, 4, "RECTANGLE");
    old.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        instance(3, 2, 4, vec![]),
        node(4, 0, "SYMBOL"),
        old,
        node(6, 0, "SYMBOL"),
        node(7, 6, "ELLIPSE"),
        instance(
            10,
            1,
            2,
            vec![patch(&[3], &[("overriddenSymbolID", id(6).to_val())])],
        ),
    ]);
    let walk = resolve_instance(&doc, id(10)).unwrap();
    let ids: Vec<String> = walk.nodes.iter().map(|r| r.path.to_string()).collect();
    assert_eq!(ids, ["I99:10;99:3", "I99:10;99:3;99:7"]);
    assert_eq!(walk.nodes[1].node, id(7));
    assert!(
        resolve_path(&doc, &path(&[10, 3, 5]))
            .unwrap_err()
            .is_not_found()
    );
}

#[test]
fn an_override_reaches_two_nested_instances_deep() {
    let mut rect = node(7, 6, "RECTANGLE");
    rect.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        node(20, 2, "FRAME"),
        instance(
            3,
            20,
            4,
            vec![patch(
                &[5, 7],
                &[
                    ("fillPaints", paint(0.0, 1.0, 0.0)),
                    ("strokeWeight", Val::Float(3.0)),
                ],
            )],
        ),
        node(4, 0, "SYMBOL"),
        instance(5, 4, 6, vec![]),
        node(6, 0, "SYMBOL"),
        rect,
        instance(
            10,
            1,
            2,
            vec![patch(&[3, 5, 7], &[("fillPaints", paint(0.0, 0.0, 1.0))])],
        ),
    ]);
    let rect = at(&doc, &[10, 3, 5, 7]);
    assert_eq!(
        rect.get("fillPaints"),
        Some(&paint(0.0, 0.0, 1.0)),
        "outer override wins"
    );
    assert_eq!(
        rect.num("strokeWeight"),
        Some(3.0),
        "inner override still applies"
    );
    assert_eq!(
        pixel(&pixels(&doc), 20, 20),
        &[0, 0, 255, 255],
        "renderer agrees"
    );
    let walk = resolve_instance(&doc, id(10)).unwrap();
    let levels: Vec<usize> = walk.nodes.iter().map(|r| r.level).collect();
    assert_eq!(levels, [1, 2, 3, 4], "frame, instance, instance, rectangle");
}

#[test]
fn unknown_paths_and_cycles_are_errors_not_panics() {
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        node(3, 2, "RECTANGLE"),
        instance(4, 2, 2, vec![]),
        instance(10, 1, 2, vec![]),
    ]);
    for ids in [&[10, 8][..], &[10, 3, 3], &[11, 3], &[3, 3], &[1, 3]] {
        assert!(resolve_path(&doc, &path(ids)).is_err(), "resolved {ids:?}");
    }
    assert!(
        resolve_path(&doc, &path(&[10, 8]))
            .unwrap_err()
            .is_not_found()
    );
    let walk = resolve_instance(&doc, id(10)).unwrap();
    assert_eq!(
        walk.nodes.len(),
        2,
        "the self-instance is listed but not expanded"
    );
    assert_eq!(walk.skipped_instances, 1);
}

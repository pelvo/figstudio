mod common;
use common::*;
use figstudio_fig::{
    Document, Val,
    resolve::{self, InstancePath},
};
use figstudio_render::{Renderer, kurbo::Point};

#[test]
fn one_focus_lays_out_all_siblings_with_nested_resize_rotation_and_scale() {
    let mut child = node(4, 3, "RECTANGLE");
    child.data.set("size", size(10.0, 10.0));
    child
        .data
        .set("horizontalConstraint", Val::Str("CENTER".into()));
    child
        .data
        .set("verticalConstraint", Val::Str("CENTER".into()));
    child.data.set(
        "transform",
        obj(&[
            ("m00", Val::Float(0.0)),
            ("m01", Val::Float(-1.0)),
            ("m10", Val::Float(1.0)),
            ("m11", Val::Float(0.0)),
            ("m02", Val::Float(20.0)),
            ("m12", Val::Float(10.0)),
        ]),
    );
    let mut top = instance(8, 1, 7, vec![]);
    top.data
        .get_mut("symbolData")
        .unwrap()
        .set("uniformScaleFactor", Val::Float(2.0));
    let mut hidden = node(6, 3, "RECTANGLE");
    hidden.data.set("visible", Val::Bool(false));
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        node(3, 2, "FRAME"),
        child,
        hidden,
        instance(5, 7, 2, vec![patch(&[3], &[("size", size(80.0, 80.0))])]),
        node(7, 0, "SYMBOL"),
        top,
    ]);
    let mut renderer = Renderer::new();
    let parent = InstancePath::new(vec![id(8), id(5), id(3)]).unwrap();
    let layouts = renderer.child_layouts(&doc, id(8), Some(&parent), 10);
    assert_eq!(layouts.len(), 2);
    assert_eq!(renderer.inspection_work().focus_walks, 1);
    assert_eq!(renderer.inspection_work().definitions_builds, 1);
    for layout in &layouts {
        if layout.data.flag("visible") == Some(false) {
            assert!(renderer.path_layout(&doc, &layout.path).is_none());
            continue;
        }
        assert_eq!(
            renderer.path_layout(&doc, &layout.path),
            Some((layout.transform, layout.data.clone()))
        );
        assert_eq!(
            renderer.path_bounds(&doc, &layout.path),
            Some(layout.bounds)
        );
    }
    let rect = layouts.iter().find(|l| l.node == id(4)).unwrap();
    assert_eq!(
        rect.transform * Point::new(0.0, 0.0),
        Point::new(80.0, 60.0)
    );
    assert_eq!(
        renderer.child_layouts(&doc, id(8), Some(&parent), 1).len(),
        1
    );
    assert!(
        renderer
            .child_layouts(&doc, id(8), Some(&parent), 0)
            .is_empty()
    );
    assert!(renderer.child_layouts(&doc, id(3), None, 10).is_empty());
    assert!(renderer.child_layouts(&doc, id(99), None, 10).is_empty());
}

#[test]
fn child_layout_matches_path_inspection_on_every_public_fixture() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut files: Vec<_> = std::fs::read_dir(fixtures)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "fig"))
        .collect();
    files.sort();
    assert_eq!(files.len(), 7);
    let mut compared = 0;
    for file in files {
        let doc = Document::open(&file).unwrap();
        let mut renderer = Renderer::new();
        let definitions = renderer.definitions(&doc);
        for instance in doc.nodes.iter().filter(|n| n.kind == "INSTANCE") {
            let mut pending = vec![None];
            while let Some(parent) = pending.pop() {
                for layout in renderer.child_layouts(&doc, instance.id, parent.as_ref(), 2048) {
                    assert_eq!(
                        resolve::tree_path(&doc, &layout.path).unwrap(),
                        resolve::tree_path_with(&doc, &definitions, &layout.path).unwrap()
                    );
                    let plain = resolve::resolve_path(&doc, &layout.path).unwrap();
                    let shared =
                        resolve::resolve_path_with(&doc, &definitions, &layout.path).unwrap();
                    assert_eq!(plain.data, shared.data);
                    if layout.data.flag("visible") == Some(false) {
                        assert!(renderer.path_layout(&doc, &layout.path).is_none());
                        continue;
                    }
                    assert_eq!(
                        renderer.path_layout(&doc, &layout.path),
                        Some((layout.transform, layout.data.clone()))
                    );
                    assert_eq!(
                        renderer.path_bounds(&doc, &layout.path),
                        Some(layout.bounds)
                    );
                    compared += 1;
                    pending.push(Some(layout.path));
                }
            }
        }
        assert_eq!(renderer.inspection_work().definitions_builds, 1);
    }
    assert!(compared > 0);
}

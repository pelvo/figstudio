//! Work counts, not timing: stationary canvas frames must not walk the instance again.
use super::inner_fixture::{document, id, path};
use crate::{canvas_pick, tab::Tab};
use figstudio_fig::{Document, Val};
use figstudio_render::kurbo::Point;
use std::{sync::Arc, time::Duration};

fn tab(doc: Document) -> Tab {
    let mut tab = Tab::new(
        1,
        "synthetic.fig".into(),
        Arc::new(doc),
        None,
        Duration::ZERO,
    );
    tab.hover = Some(id(200));
    tab.selection = tab.hover;
    tab.inner = Some(path(&[103]));
    tab
}

#[test]
fn click_builds_indexes_once_and_stationary_hover_and_outline_do_no_walks() {
    let mut tab = tab(document());
    let point = Point::new(30.0, 30.0);
    canvas_pick::click(&mut tab, &[id(200)], Some(point), true, false);
    assert_eq!(tab.inner, Some(path(&[103])));
    let clicked = tab.pick_cache.work(&tab.doc);
    assert_eq!(clicked.definitions_builds, 1);
    assert!(clicked.focus_walks > 0);
    let hover = canvas_pick::hover(&tab, Some(point), false);
    assert_eq!(hover, tab.inner);
    let warmed = tab.pick_cache.work(&tab.doc);
    for _ in 0..10 {
        assert_eq!(canvas_pick::hover(&tab, Some(point), false), hover);
    }
    assert_eq!(tab.pick_cache.work(&tab.doc), warmed);
    let corners = tab
        .pick_cache
        .corners(&tab.doc, tab.inner.as_ref().unwrap());
    assert!(corners.is_some());
    let outlined = tab.pick_cache.work(&tab.doc);
    assert_eq!(outlined.focus_walks, warmed.focus_walks + 1);
    for _ in 0..10 {
        assert_eq!(
            tab.pick_cache
                .corners(&tab.doc, tab.inner.as_ref().unwrap()),
            corners
        );
    }
    assert_eq!(tab.pick_cache.work(&tab.doc), outlined);
}

#[test]
fn hover_key_tracks_pixels_zoom_deep_selection_and_inner_path() {
    let mut tab = tab(document());
    let point = Point::new(130.0, 30.0);
    assert_eq!(
        canvas_pick::hover(&tab, Some(point), false),
        Some(path(&[104]))
    );
    let work = tab.pick_cache.work(&tab.doc);
    canvas_pick::hover(&tab, Some(Point::new(130.2, 30.2)), false);
    assert_eq!(tab.pick_cache.work(&tab.doc), work);
    for change in 0..5 {
        let before = tab.pick_cache.work(&tab.doc);
        match change {
            0 => {
                tab.view.zoom = 2.0;
            }
            1 => {
                tab.selection = None;
            }
            2 => {
                tab.inner = Some(path(&[104]));
            }
            _ => {}
        }
        let deep = change != 4;
        let at = if change == 3 {
            Point::new(131.0, 30.0)
        } else {
            point
        };
        if !deep {
            tab.selection = tab.hover;
        }
        let target = canvas_pick::hover(&tab, Some(at), deep);
        assert_eq!(target, Some(path(&[if deep { 105 } else { 104 }])));
        assert!(tab.pick_cache.work(&tab.doc).focus_walks > before.focus_walks);
        assert_eq!(tab.pick_cache.work(&tab.doc).definitions_builds, 1);
    }
}

#[test]
fn document_identity_invalidates_indexes_hover_and_cached_outline_misses() {
    let mut tab = tab(document());
    let point = Point::new(30.0, 30.0);
    let selected = path(&[103]);
    assert_eq!(
        canvas_pick::hover(&tab, Some(point), true),
        Some(selected.clone())
    );
    assert!(tab.pick_cache.corners(&tab.doc, &selected).is_some());
    let old_index = tab
        .pick_cache
        .with(&tab.doc, |_, definitions| definitions.clone());
    let mutate = |tab: &mut Tab, shown| {
        let doc = Arc::get_mut(&mut tab.doc).unwrap();
        let mut nodes = doc.nodes.clone();
        nodes
            .iter_mut()
            .find(|n| n.id == id(200))
            .unwrap()
            .data
            .set("visible", Val::Bool(shown));
        doc.set_nodes(nodes).unwrap();
    };
    mutate(&mut tab, false);
    assert_eq!(canvas_pick::hover(&tab, Some(point), true), None);
    assert_eq!(tab.pick_cache.corners(&tab.doc, &selected), None);
    let new_index = tab
        .pick_cache
        .with(&tab.doc, |_, definitions| definitions.clone());
    assert!(!Arc::ptr_eq(&old_index, &new_index));
    assert_eq!(tab.pick_cache.work(&tab.doc).definitions_builds, 1);
    mutate(&mut tab, true);
    assert_eq!(
        canvas_pick::hover(&tab, Some(point), true),
        Some(selected.clone())
    );
    assert!(tab.pick_cache.corners(&tab.doc, &selected).is_some());
}

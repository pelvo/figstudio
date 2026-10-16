mod support;

use figstudio_diff::{Category, PageStatus, Summary, diff_documents};
use figstudio_fig::{Guid, Val};
use support::{document, node, object, rectangle};

#[test]
fn identical_pair_is_empty() {
    let doc = document(vec![rectangle(2, 1)]);
    let report = diff_documents(&doc, &doc);
    assert!(report.pages.is_empty());
    assert_eq!(report.summary, Summary::default());
    assert!(report.complete);
    assert!(report.truncated.is_empty());
    assert_eq!(report.old.node_count, 3);
    assert_eq!(report.old.page_count, 1);
    assert_eq!(report.new.node_count, 3);
    assert_eq!(report.new.page_count, 1);
}

#[test]
fn page_added_and_removed() {
    let old = document(vec![node(3, Some(0), "CANVAS", "b"), rectangle(4, 3)]);
    let new = document(vec![node(5, Some(0), "CANVAS", "c"), rectangle(6, 5)]);
    let report = diff_documents(&old, &new);
    assert_eq!(
        report.pages.iter().map(|p| p.id).collect::<Vec<_>>(),
        [Guid(0, 3), Guid(0, 5)]
    );
    assert_eq!(report.pages[0].status, PageStatus::Removed);
    assert_eq!(report.pages[0].old_name.as_deref(), Some("layer-3"));
    assert_eq!(report.pages[0].new_name, None);
    assert_eq!(report.pages[1].status, PageStatus::Added);
    assert_eq!(report.pages[1].old_name, None);
    assert_eq!(report.pages[1].new_name.as_deref(), Some("layer-5"));
    assert_eq!(report.pages[0].removed[0].id, Guid(0, 4));
    assert_eq!(report.pages[1].added[0].id, Guid(0, 6));
    assert_eq!(report.summary.pages_added, 1);
    assert_eq!(report.summary.pages_removed, 1);
    assert_eq!(report.summary.nodes_added, 1);
    assert_eq!(report.summary.nodes_removed, 1);
}

#[test]
fn node_added_subtree_is_one_row() {
    let old = document(vec![]);
    let new = document(vec![
        node(2, Some(1), "FRAME", "a"),
        rectangle(3, 2),
        rectangle(4, 3),
    ]);
    let report = diff_documents(&old, &new);
    assert_eq!(report.pages[0].added.len(), 1);
    assert_eq!(report.pages[0].added[0].id, Guid(0, 2));
    assert_eq!(report.pages[0].added[0].descendants, 2);
    assert_eq!(
        report.pages[0].added[0].path,
        ["layer-0".to_string(), "layer-1".to_string()]
    );
    assert_eq!(report.summary.nodes_added, 3);
}

#[test]
fn page_rename_records_names() {
    let mut old_page = node(3, Some(0), "CANVAS", "b");
    old_page.data.set("name", Val::Str("before".into()));
    old_page.name = "before".into();
    let mut new_page = node(3, Some(0), "CANVAS", "b");
    new_page.data.set("name", Val::Str("after".into()));
    new_page.name = "after".into();
    let report = diff_documents(&document(vec![old_page]), &document(vec![new_page]));
    let page = report.pages.iter().find(|p| p.id == Guid(0, 3)).unwrap();
    assert_eq!(page.old_name.as_deref(), Some("before"));
    assert_eq!(page.new_name.as_deref(), Some("after"));
}

#[test]
fn node_removed_subtree_is_one_row() {
    let old = document(vec![rectangle(2, 1), rectangle(3, 2)]);
    let new = document(vec![]);
    let report = diff_documents(&old, &new);
    assert_eq!(report.pages[0].removed.len(), 1);
    assert_eq!(report.pages[0].removed[0].descendants, 1);
    assert_eq!(report.summary.nodes_removed, 2);
}

#[test]
fn properties_descend_to_leaves_and_preserve_absence() {
    let mut before = rectangle(2, 1);
    before.data.set(
        "fillPaints",
        Val::Arr(vec![object(&[(
            "color",
            object(&[("r", Val::Float(0.25))]),
        )])]),
    );
    let mut after = before.clone();
    after.data.set(
        "fillPaints",
        Val::Arr(vec![object(&[(
            "color",
            object(&[("r", Val::Float(0.75))]),
        )])]),
    );
    after.data.set("visible", Val::Bool(false));
    let report = diff_documents(&document(vec![before]), &document(vec![after]));
    let changes = &report.pages[0].changed[0].changes;
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].field, "fillPaints.0.color.r");
    assert_eq!(changes[0].category, Category::Fill);
    assert_eq!(changes[0].old, Some(Val::Float(0.25)));
    assert_eq!(changes[0].new, Some(Val::Float(0.75)));
    assert_eq!(changes[1].field, "visible");
    assert_eq!(changes[1].old, None);
    assert_eq!(changes[1].new, Some(Val::Bool(false)));
    assert_eq!(report.summary.properties_changed, 2);
}

#[test]
fn text_and_font_change() {
    let mut before = node(2, Some(1), "TEXT", "a");
    before.data.set(
        "textData",
        object(&[("characters", Val::Str("old".into()))]),
    );
    before
        .data
        .set("fontName", object(&[("family", Val::Str("serif".into()))]));
    let mut after = before.clone();
    after.data.set(
        "textData",
        object(&[("characters", Val::Str("new".into()))]),
    );
    after.data.set(
        "fontName",
        object(&[("family", Val::Str("sans-serif".into()))]),
    );
    after.data.set("fontSize", Val::Float(18.0));
    let report = diff_documents(&document(vec![before]), &document(vec![after]));
    let diff = &report.pages[0].changed[0];
    let text = diff.text.as_ref().unwrap();
    assert_eq!((&*text.old, &*text.new), ("old", "new"));
    assert_eq!(
        diff.changes
            .iter()
            .filter(|p| p.category == Category::Font)
            .count(),
        2
    );
    assert!(diff.changes.iter().any(|p| p.category == Category::Text));
}

#[test]
fn cross_page_move_is_not_add_remove() {
    let old = document(vec![node(3, Some(0), "CANVAS", "b"), rectangle(2, 1)]);
    let new = document(vec![node(3, Some(0), "CANVAS", "b"), rectangle(2, 3)]);
    let report = diff_documents(&old, &new);
    assert_eq!(report.summary.nodes_added, 0);
    assert_eq!(report.summary.nodes_removed, 0);
    assert_eq!(report.summary.nodes_moved, 1);
    assert_eq!(report.summary.nodes_changed, 1);
    let destination = report.pages.iter().find(|p| p.id == Guid(0, 3)).unwrap();
    let moved = destination.changed[0].moved.as_ref().unwrap();
    assert_eq!(moved.old_parent, Some(Guid(0, 1)));
    assert_eq!(moved.new_parent, Some(Guid(0, 3)));
    assert_eq!(report.summary.pages_changed, 2);
}

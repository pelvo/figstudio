mod support;

use figstudio_diff::{PageStatus, diff_documents, render_html};
use figstudio_fig::{Guid, Val};
use support::{document, node, object, rectangle};

#[test]
fn move_to_document_root_keeps_source_detail_without_moved_out_note() {
    let old = document(vec![rectangle(2, 1)]);
    let new = document(vec![rectangle(2, 0)]);
    let report = diff_documents(&old, &new);
    let source = report.pages.iter().find(|p| p.id == Guid(0, 1)).unwrap();
    assert!(source.moved_out.is_empty());
    assert_eq!(source.changed.len(), 1);
    assert_eq!(source.changed[0].id, Guid(0, 2));
    assert!(source.changed[0].moved.is_some());
    assert_eq!(report.summary.nodes_moved, 1);
    let html = render_html(&report);
    let source_html = html.split("</section>").next().unwrap();
    assert!(!source_html.contains("Moved out:"));
}

#[test]
fn surviving_subtree_leaves_a_removed_page_without_becoming_removed() {
    let old = document(vec![
        node(3, Some(0), "CANVAS", "b"),
        rectangle(2, 3),
        rectangle(4, 2),
    ]);
    let new = document(vec![rectangle(2, 1), rectangle(4, 2)]);
    let report = diff_documents(&old, &new);
    assert_eq!(report.summary.pages_removed, 1);
    assert_eq!(report.summary.nodes_removed, 0);
    assert_eq!(report.summary.nodes_moved, 1);
    assert_eq!(report.summary.nodes_changed, 1);
    assert!(
        report
            .pages
            .iter()
            .find(|p| p.status == PageStatus::Removed)
            .unwrap()
            .removed
            .is_empty()
    );
    assert_eq!(
        report
            .pages
            .iter()
            .find(|p| p.id == Guid(0, 1))
            .unwrap()
            .changed[0]
            .id,
        Guid(0, 2)
    );
}

#[test]
fn surviving_subtree_enters_added_page_without_becoming_added() {
    let old = document(vec![rectangle(2, 1), rectangle(4, 2)]);
    let new = document(vec![
        node(3, Some(0), "CANVAS", "b"),
        rectangle(2, 3),
        rectangle(4, 2),
    ]);
    let report = diff_documents(&old, &new);
    assert_eq!(report.summary.pages_added, 1);
    assert_eq!(report.summary.nodes_added, 0);
    assert_eq!(report.summary.nodes_moved, 1);
    let page = report
        .pages
        .iter()
        .find(|p| p.status == PageStatus::Added)
        .unwrap();
    assert!(page.added.is_empty());
    assert_eq!(page.changed[0].id, Guid(0, 2));
}

#[test]
fn removed_parent_and_removed_child_under_moved_survivor_are_not_lost() {
    let old = document(vec![rectangle(2, 1), rectangle(3, 2), rectangle(4, 3)]);
    let new = document(vec![rectangle(3, 1)]);
    let report = diff_documents(&old, &new);
    assert_eq!(report.summary.nodes_removed, 2);
    assert_eq!(report.summary.nodes_moved, 1);
    assert_eq!(
        report.pages[0]
            .removed
            .iter()
            .map(|n| n.id)
            .collect::<Vec<_>>(),
        [Guid(0, 2), Guid(0, 4)]
    );
    assert_eq!(report.pages[0].removed[0].descendants, 2);
}

#[test]
fn character_style_assignments_are_not_hidden_when_table_is_unchanged() {
    let mut before = node(2, Some(1), "TEXT", "a");
    before.data.set(
        "textData",
        object(&[
            ("characters", Val::Str("ab".into())),
            (
                "characterStyleIDs",
                Val::Arr(vec![Val::Int(0), Val::Int(1)]),
            ),
        ]),
    );
    let mut after = before.clone();
    after.data.set(
        "textData",
        object(&[
            ("characters", Val::Str("ab".into())),
            (
                "characterStyleIDs",
                Val::Arr(vec![Val::Int(1), Val::Int(0)]),
            ),
        ]),
    );
    let report = diff_documents(&document(vec![before]), &document(vec![after]));
    let diff = &report.pages[0].changed[0];
    assert_eq!(diff.changes.len(), 2);
    assert_eq!(diff.changes[0].field, "textData.characterStyleIDs.0");
    assert!(diff.text.is_some());
}

#[test]
fn root_and_orphan_changes_are_counted_as_incomplete() {
    let old = document(vec![rectangle(2, 99)]);
    let mut orphan = rectangle(2, 99);
    orphan.data.set("visible", Val::Bool(false));
    let report = diff_documents(&old, &document(vec![orphan]));
    assert_eq!(report.summary.nodes_changed, 1);
    assert_eq!(report.summary.properties_changed, 1);
    assert!(!report.complete);
    assert_eq!(report.truncated.len(), 1);
    assert!(report.truncated[0].starts_with("1 added, removed or changed nodes"));
}

#[test]
fn ungrouped_added_and_removed_nodes_are_counted_as_incomplete() {
    let old = document(vec![rectangle(2, 99)]);
    let report = diff_documents(&old, &document(vec![]));
    assert_eq!(report.summary.nodes_removed, 1);
    assert!(!report.complete);

    let report = diff_documents(&document(vec![]), &document(vec![rectangle(2, 99)]));
    assert_eq!(report.summary.nodes_added, 1);
    assert!(!report.complete);
    assert_eq!(report.truncated.len(), 1);
}

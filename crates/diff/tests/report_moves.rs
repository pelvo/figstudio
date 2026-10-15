mod support;

use figstudio_diff::diff_documents;
use figstudio_fig::Val;
use support::{document, node, object, rectangle};

#[test]
fn moves_replace_parent_index_deltas_without_hiding_other_changes() {
    let before = rectangle(2, 1);
    let mut after = node(2, Some(3), "RECTANGLE", "b");
    after
        .data
        .set("visible", object(&[("value", Val::Bool(false))]));
    let report = diff_documents(
        &document(vec![before, rectangle(3, 1)]),
        &document(vec![after, rectangle(3, 1)]),
    );
    let change = &report.pages[0].changed[0];
    assert!(change.moved.is_some());
    assert_eq!(change.changes.len(), 1);
    assert_eq!(change.changes[0].field, "visible.value");
    assert_eq!(report.summary.properties_changed, 1);
}

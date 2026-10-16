mod support;

use figstudio_diff::{Category, PageStatus, diff_documents};
use figstudio_fig::{Guid, Val};
use support::{document, node, object, rectangle};

#[test]
fn storage_order_does_not_change_output_or_create_moves() {
    let first = node(2, Some(1), "RECTANGLE", "a");
    let last = node(3, Some(1), "RECTANGLE", "b");
    let old = document(vec![last.clone(), first.clone()]);
    let reordered = document(vec![first.clone(), last.clone()]);
    assert!(diff_documents(&old, &reordered).pages.is_empty());
    let empty = document(vec![]);
    let a = diff_documents(&empty, &old);
    let b = diff_documents(&empty, &reordered);
    assert_eq!(a, b);
    assert_eq!(
        a.pages[0].added.iter().map(|n| n.id).collect::<Vec<_>>(),
        [Guid(0, 2), Guid(0, 3)]
    );
}

#[test]
fn position_change_is_a_move() {
    let old = document(vec![node(2, Some(1), "RECTANGLE", "a")]);
    let new = document(vec![node(2, Some(1), "RECTANGLE", "z")]);
    let report = diff_documents(&old, &new);
    let moved = report.pages[0].changed[0].moved.as_ref().unwrap();
    assert_eq!(moved.old_position, "a");
    assert_eq!(moved.new_position, "z");
    assert_eq!(report.summary.nodes_moved, 1);
    assert_eq!(report.summary.nodes_added + report.summary.nodes_removed, 0);
}

#[test]
fn internal_page_and_page_fields_are_compared() {
    let mut page = node(3, Some(0), "CANVAS", "b");
    page.data.set("internalOnly", Val::Bool(true));
    let old = document(vec![page.clone()]);
    page.data
        .set("backgroundColor", object(&[("r", Val::Float(0.5))]));
    let report = diff_documents(&old, &document(vec![page]));
    assert_eq!(report.pages[0].id, Guid(0, 3));
    assert_eq!(report.pages[0].status, PageStatus::Changed);
    assert_eq!(
        report.pages[0].changed[0].changes[0].field,
        "backgroundColor.r"
    );
    assert!(report.complete);
}

#[test]
fn font_only_and_stored_style_changes_keep_text_context() {
    let mut before = node(2, Some(1), "TEXT", "a");
    before.data.set(
        "textData",
        object(&[
            ("characters", Val::Str("same".into())),
            (
                "styleOverrideTable",
                Val::Arr(vec![object(&[("fontSize", Val::Float(12.0))])]),
            ),
        ]),
    );
    let mut after = before.clone();
    after.data.set(
        "textData",
        object(&[
            ("characters", Val::Str("same".into())),
            (
                "styleOverrideTable",
                Val::Arr(vec![object(&[("fontSize", Val::Float(16.0))])]),
            ),
        ]),
    );
    let report = diff_documents(&document(vec![before]), &document(vec![after]));
    let text = report.pages[0].changed[0].text.as_ref().unwrap();
    assert_eq!(text.old, "same");
    assert_eq!(text.new, "same");
    assert_eq!(text.runs.len(), 1);
    assert_eq!(text.runs[0].index, 0);
    assert_eq!(text.runs[0].changes[0].category, Category::Font);
}

#[test]
fn removed_arrays_unknown_fields_and_empty_objects_preserve_values() {
    let mut before = rectangle(2, 1);
    before.data.set(
        "unknown",
        Val::Arr(vec![object(&[("flag", Val::Bool(true))])]),
    );
    let mut after = rectangle(2, 1);
    after.data.set("empty", object(&[]));
    let report = diff_documents(&document(vec![before]), &document(vec![after]));
    let changes = &report.pages[0].changed[0].changes;
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].field, "empty");
    assert_eq!(changes[0].old, None);
    assert_eq!(changes[0].new, Some(object(&[])));
    assert_eq!(changes[1].field, "unknown.0.flag");
    assert_eq!(changes[1].old, Some(Val::Bool(true)));
    assert_eq!(changes[1].new, None);
    assert_eq!(changes[1].category, Category::Other);
}

#[test]
fn equal_nan_bits_are_not_changes() {
    let mut layer = rectangle(2, 1);
    layer
        .data
        .set("opacity", Val::Float(f64::from_bits(0x7ff8_0000_0000_0001)));
    let a = document(vec![layer.clone()]);
    let b = document(vec![layer]);
    assert!(diff_documents(&a, &b).pages.is_empty());
}

#[test]
fn moved_child_of_added_frame_stays_a_move() {
    let old = document(vec![rectangle(2, 1)]);
    let new = document(vec![node(3, Some(1), "FRAME", "a"), rectangle(2, 3)]);
    let report = diff_documents(&old, &new);
    assert_eq!(report.summary.nodes_added, 1);
    assert_eq!(report.summary.nodes_moved, 1);
    assert_eq!(report.pages[0].added.len(), 1);
    assert_eq!(report.pages[0].changed[0].id, Guid(0, 2));
}

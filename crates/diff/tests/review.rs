mod support;

use figstudio_diff::{diff_documents, render_html};
use figstudio_fig::{Guid, Node, Val};
use support::{document, node, object, rectangle, two_page_document};

#[test]
fn cross_page_move_keeps_source_note_without_double_counting() {
    let old = two_page_document(vec![rectangle(2, 1)], vec![]);
    let new = two_page_document(vec![], vec![rectangle(2, 11)]);
    let report = diff_documents(&old, &new);
    let json: serde_json::Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
    assert_eq!(json["pages"][0]["moved_out"], serde_json::json!([[0, 2]]));
    assert_eq!(report.pages[0].moved_out, vec![Guid(0, 2)]);
    assert_eq!(report.pages[1].changed[0].id, Guid(0, 2));
    assert_eq!(report.summary.nodes_moved, 1);
    assert_eq!(report.summary.nodes_changed, 1);
    assert_eq!(report.summary.pages_changed, 2);
    let html = render_html(&report);
    assert!(!html.contains("<section"), "moves alone show no page");
    assert!(html.contains("2 pages with no visible change"));
    assert!(
        !html.contains("layers moved without content change"),
        "{html}"
    );
}

#[test]
fn html_prints_style_run_changes_once_and_keeps_flat_json() {
    let build = |size| {
        let mut text = node(2, Some(1), "TEXT", "a");
        text.data.set(
            "textData",
            object(&[
                ("characters", Val::Str("sample".into())),
                (
                    "styleOverrideTable",
                    Val::Arr(vec![object(&[("fontSize", Val::Float(size))])]),
                ),
            ]),
        );
        document(vec![Node::from_data(text.data).unwrap()])
    };
    let report = diff_documents(&build(12.0), &build(14.0));
    let html = render_html(&report);
    assert_eq!(html.matches("fontSize").count(), 1, "{html}");
    assert!(
        html.contains("fontSize:</span><span class=\"o\">12</span><span class=\"ar\">→</span><span class=\"n\">14</span>"),
        "{html}"
    );
    assert!(
        report
            .to_json()
            .unwrap()
            .contains("textData.styleOverrideTable.0.fontSize")
    );
}

#[test]
fn html_labels_coverage_and_added_layers_precisely() {
    let report = diff_documents(
        &document(vec![]),
        &document(vec![rectangle(2, 1), rectangle(3, 2)]),
    );
    let html = render_html(&report);
    assert!(html.contains("class=\"sg\">+</span><span class=\"hd\"><span class=\"nm\">layer-2</span><span class=\"kd\">RECTANGLE</span>"), "{html}");
    assert!(html.contains("<b>Coverage</b><span>complete</span>"));
}

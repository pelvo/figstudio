mod support;

use figstudio_diff::{DiffReport, RenderDiff, diff_documents, render_html};
use figstudio_fig::{Node, Val};
use support::{document, node, object, rectangle};

fn pair() -> DiffReport {
    let before = node(2, Some(1), "TEXT", "a");
    let mut after = before.clone();
    after
        .data
        .set("name", Val::Str("<script>alert('x')</script>&\"".into()));
    after.data.set(
        "textData",
        object(&[("characters", Val::Str("<b>new</b>".into()))]),
    );
    let after = Node::from_data(after.data).unwrap();
    let mut report = diff_documents(
        &document(vec![before, rectangle(3, 1)]),
        &document(vec![after, rectangle(4, 1)]),
    );
    report.old.path_label = "old.fig".into();
    report.new.path_label = "new.fig".into();
    report
}

#[test]
fn json_golden_pipeline_and_round_trip() {
    let report = pair();
    let json = report.to_json().unwrap();
    assert!(json.starts_with("{\"old\":{\"path_label\":\"old.fig\",\"format_version\":"));
    assert!(json.contains(r#""summary":{"pages_added":0,"pages_removed":0,"pages_changed":1,"nodes_added":1,"nodes_removed":1,"nodes_changed":1,"nodes_moved":0,"properties_changed":2}"#));
    assert!(json.contains(
        r#""field":"textData.characters","old":null,"new":"<b>new</b>","category":"Text""#
    ));
    assert!(!json.contains("\"children\""));
    assert_eq!(serde_json::from_str::<DiffReport>(&json).unwrap(), report);
}

#[test]
fn json_and_html_are_byte_deterministic() {
    let a = pair();
    let b = pair();
    assert_eq!(a.to_json().unwrap(), b.to_json().unwrap());
    assert_eq!(render_html(&a), render_html(&b));
}

#[test]
fn html_escapes_all_names_values_paths_labels_and_coverage_messages() {
    let mut report = pair();
    report.old.path_label = "<script>old</script>".into();
    report.pages[0].new_name = Some("<script>page</script>".into());
    report.pages[0].added[0].path = vec!["<script>ancestor</script>".into()];
    report.complete = false;
    report.truncated = vec!["<script>coverage</script>".into()];
    let html = render_html(&report);
    assert!(!html.contains("<script>"));
    for escaped in [
        "&lt;script&gt;old",
        "&lt;script&gt;page",
        "&lt;script&gt;ancestor",
        "&lt;script&gt;coverage",
        "&lt;b&gt;new&lt;/b&gt;",
    ] {
        assert!(html.contains(escaped), "missing {escaped}");
    }
    assert!(html.contains("Content-Security-Policy"));
    assert!(html.contains("default-src 'none'"));
    assert!(!html.contains("src="));
    assert!(!html.contains("<script"));
}

#[test]
fn html_has_page_sections_counts_absence_and_text_rows() {
    let html = render_html(&pair());
    for expected in [
        "old.fig",
        "new.fig",
        "<section",
        "layer-1",
        "Added",
        "Removed",
        "Changed",
        "Nodes: 1 added, 1 removed, 1 changed, 0 moved",
        "Properties: 2",
        "Stored-change coverage: complete",
        "textData.characters: (absent)",
        "Text:",
        "layer-3",
        "layer-4",
    ] {
        assert!(html.contains(expected), "missing {expected}");
    }
    let mut report = pair();
    report.complete = false;
    report.truncated.push("Ungrouped changes".into());
    let html = render_html(&report);
    assert!(html.contains("Stored-change coverage: incomplete"));
    assert!(html.contains("Ungrouped changes"));
}

#[test]
fn json_preserves_nonfinite_changes() {
    let mut report = pair();
    let nan = Val::Float(f64::NAN);
    report.pages[0].changed[0].changes[0].new = Some(Val::Arr(vec![nan.clone()]));
    report.pages[0].changed[0].changes[1].new = Some(object(&[("x", nan.clone())]));
    let json = report.to_json().unwrap();
    let parsed: DiffReport = serde_json::from_str(&json).unwrap();
    for (a, b) in [
        (
            &parsed.pages[0].changed[0].changes[0].new,
            &report.pages[0].changed[0].changes[0].new,
        ),
        (
            &parsed.pages[0].changed[0].changes[1].new,
            &report.pages[0].changed[0].changes[1].new,
        ),
    ] {
        let (Val::Float(parsed_nan), Val::Float(want)) = (
            nested_float(a.as_ref().unwrap()),
            nested_float(b.as_ref().unwrap()),
        ) else {
            panic!("expected nested floats");
        };
        assert_eq!(parsed_nan.to_bits(), want.to_bits());
    }
}

fn nested_float(value: &Val) -> &Val {
    match value {
        Val::Arr(items) => &items[0],
        Val::Obj(obj) => obj.get("x").unwrap(),
        _ => unreachable!(),
    }
}

#[test]
fn html_links_only_relative_render_paths_and_omits_missing_images() {
    let mut report = pair();
    assert!(!render_html(&report).contains("<a "));
    report.pages[0].render = Some(RenderDiff {
        old_png: Some("pages/before & after.png".into()),
        new_png: Some("javascript:alert(1)".into()),
        diff_png: None,
        pixels_changed: Some(42),
        worst_channel_delta: Some(7),
        width: Some(3),
        height: Some(2),
        ..RenderDiff::default()
    });
    let html = render_html(&report);
    assert!(html.contains("href=\"./pages/before &amp; after.png\""));
    assert!(
        html.contains("<p>42 pixels changed, worst channel delta 7, 3x2</p>"),
        "{html}"
    );
    assert!(!html.contains("javascript:"));
    assert!(!html.contains("Diff PNG"));
    for invalid in [
        "../private.png",
        "/absolute.png",
        "//host/image.png",
        "pages\\x.png",
        "pages/x.png#fragment",
        "%2e%2e/private.png",
    ] {
        report.pages[0].render.as_mut().unwrap().old_png = Some(invalid.into());
        assert!(!render_html(&report).contains("<a "));
    }
}

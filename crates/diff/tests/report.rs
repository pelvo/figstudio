mod support;

use figstudio_diff::{DiffReport, Region, RenderDiff, diff_documents, render_html};
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
    assert!(!html.contains("<img"));
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
        "class=\"sg\">+</span><span class=\"hd\"><span class=\"nm\">layer-4</span><span class=\"kd\">RECTANGLE",
        "class=\"sg\">-</span><span class=\"hd\"><span class=\"nm\">layer-3</span>",
        "class=\"sg\">~</span><span class=\"hd\"><span class=\"nm\">&lt;script&gt;alert",
        "<b>Layers</b><span class=\"a\">+1</span>",
        "<span class=\"m\">~1</span>",
        "Full machine record: report.json",
        "<b>Coverage</b><span>complete</span>",
        "class=\"lines\"",
    ] {
        assert!(html.contains(expected), "missing {expected}");
    }
    let mut report = pair();
    report.complete = false;
    report.truncated.push("Ungrouped changes".into());
    let html = render_html(&report);
    assert!(html.contains("<b>Coverage</b><span>incomplete</span>"));
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
    assert!(!render_html(&report).contains("href=\"./"));
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
    assert!(html.contains("<img src=\"./pages/before &amp; after.png\""));
    assert!(!html.contains("<img src=\"./javascript"));
    assert!(
        html.contains("<span>42 pixels changed, worst channel delta 7, 3x2</span>"),
        "{html}"
    );
    assert!(!html.contains("javascript:"));
    assert!(!html.contains("alt=\"Diff\""));
    for invalid in [
        "../private.png",
        "/absolute.png",
        "//host/image.png",
        "pages\\x.png",
        "pages/x.png#fragment",
        "%2e%2e/private.png",
    ] {
        report.pages[0].render.as_mut().unwrap().old_png = Some(invalid.into());
        assert!(!render_html(&report).contains("href=\"./"));
    }
}

fn text_report(edit: impl Fn(&mut Val)) -> DiffReport {
    let before = node(2, Some(1), "TEXT", "a");
    let mut after = before.data.clone();
    edit(&mut after);
    let after = Node::from_data(after).unwrap();
    diff_documents(&document(vec![before]), &document(vec![after]))
}

#[test]
fn html_hides_blob_rows_but_json_keeps_them() {
    let report = text_report(|d| {
        d.set(
            "derivedTextData",
            object(&[("glyphsBlob", Val::Str("2116".into()))]),
        );
        d.set("vectorNetworkBlob", Val::Int(7));
    });
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("derivedTextData") && json.contains("vectorNetworkBlob"));
    let html = render_html(&report);
    assert!(!html.contains("derivedTextData") && !html.contains("vectorNetworkBlob"));
    // Blob-only churn is no visible change: the page is folded away, not a section.
    assert!(!html.contains("<section"), "{html}");
    assert!(html.contains("1 page with no visible change"), "{html}");
}

#[test]
fn html_groups_fill_colour_into_one_row_with_swatches() {
    let before = support::filled_rectangle(2, 1, (0.1, 0.2, 0.3));
    let after = support::filled_rectangle(2, 1, (1.0, 0.0, 0.0));
    let html = render_html(&diff_documents(
        &document(vec![before]),
        &document(vec![after]),
    ));
    assert!(
        html.contains("Fill:</span><span class=\"o\"><i class=\"sw\""),
        "{html}"
    );
    assert!(
        html.contains("#1A334D") && html.contains("#FF0000"),
        "{html}"
    );
    assert_eq!(html.matches("Fill:").count(), 1);
}

#[test]
fn html_shows_font_size_and_escaped_text_change() {
    let report = text_report(|d| {
        d.set("fontSize", Val::Int(14));
        d.set(
            "textData",
            object(&[("characters", Val::Str("<i>x</i>".into()))]),
        );
    });
    let html = render_html(&report);
    assert!(
        html.contains("fontSize:</span><span class=\"o\">(absent)</span><span class=\"ar\">→</span><span class=\"n\">14</span>"),
        "{html}"
    );
    assert!(html.contains("&lt;i&gt;x&lt;/i&gt;") && !html.contains("<i>x</i>"));
}

#[test]
fn html_csp_allows_only_same_origin_images() {
    let html = render_html(&pair());
    assert!(html.contains("img-src 'self'") && !html.contains("<script"));
}

#[test]
fn move_only_node_has_no_tilde_line_but_is_counted() {
    let before = support::filled_rectangle_at(2, 1, (0.5, 0.5, 0.5), 0.0, 0.0, 40.0, 40.0);
    let after = support::filled_rectangle_at(2, 1, (0.5, 0.5, 0.5), 30.0, 5.0, 40.0, 40.0);
    let mut kept = support::filled_rectangle(3, 1, (0.1, 0.1, 0.1));
    kept.data.set("name", Val::Str("peer".into()));
    let report = diff_documents(
        &document(vec![before, kept.clone()]),
        &document(vec![after, kept]),
    );
    assert_eq!(report.pages[0].changed.len(), 1);
    let html = render_html(&report);
    assert!(!html.contains("class=\"ln mod\""), "{html}");
    assert!(!html.contains("<section"), "{html}");
    assert!(html.contains("1 page with no visible change"), "{html}");
}

#[test]
fn newly_written_default_opacity_and_raw_override_paths_are_not_shown() {
    let report = text_report(|d| {
        d.set("opacity", Val::Float(1.0));
        d.set("visible", Val::Bool(true));
        d.set("fontSize", Val::Int(14));
    });
    let html = render_html(&report);
    assert!(
        !html.contains("class=\"k\">opacity") && !html.contains("class=\"k\">visible"),
        "{html}"
    );
    let nested = text_report(|d| {
        d.set(
            "symbolData",
            object(&[(
                "symbolOverrides",
                Val::Arr(vec![object(&[("fontSize", Val::Int(9))])]),
            )]),
        );
    });
    let html = render_html(&nested);
    assert!(!html.contains("symbolOverrides"), "{html}");
}

#[path = "report/components.rs"]
mod components;

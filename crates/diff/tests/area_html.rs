mod support;

use figstudio_diff::{Region, RenderDiff, diff_documents, render_html};
use figstudio_fig::Guid;
use support::{document, filled_rectangle};

fn report() -> figstudio_diff::DiffReport {
    let side = |color| {
        let mut frame = support::node(10, Some(1), "FRAME", "");
        frame.data.set(
            "size",
            support::object(&[
                ("x", figstudio_fig::Val::Float(200.0)),
                ("y", figstudio_fig::Val::Float(200.0)),
            ]),
        );
        document(
            std::iter::once(frame)
                .chain((2..=4).map(|id| filled_rectangle(id, 10, color)))
                .collect(),
        )
    };
    let mut report = diff_documents(&side((1.0, 0.0, 0.0)), &side((0.0, 1.0, 0.0)));
    let area = |nodes| Region {
        x: 10,
        y: 20,
        width: 30,
        height: 40,
        pixels_changed: 100,
        old_png: Some("render/zoom-before.png".into()),
        new_png: Some("render/zoom-after.png".into()),
        image_width: Some(641),
        image_height: Some(320),
        nodes,
        shared: None,
    };
    report.pages[0].render = Some(RenderDiff {
        old_png: Some("render/before.png".into()),
        new_png: Some("render/after.png".into()),
        diff_png: Some("render/diff.png".into()),
        pixels_changed: Some(900),
        width: Some(200),
        height: Some(200),
        ..Default::default()
    });
    let screen = &mut report.pages[0].screens[0];
    screen.width = 200;
    screen.height = 200;
    screen.pixels_changed = Some(300);
    screen.thumb_new = Some("render/screen-after.png".into());
    screen.regions = vec![area(vec![Guid(0, 2)]), area(vec![Guid(0, 3)]), area(vec![])];
    report
}

#[test]
fn report_places_lines_with_their_areas_and_unmatched_pictures_in_minis() {
    let report = report();
    let html = render_html(&report);
    for class in [
        "shell", "top", "brand", "files", "file", "stats", "stat", "notes", "side", "idx", "ph",
        "meta", "mr", "tool", "sec", "sh", "seg", "info", "loc", "lines", "minis", "mini", "whole",
    ] {
        assert!(
            html.contains(&format!("class=\"{class}\"")),
            "missing {class}"
        );
    }
    let area1 = html
        .split("<article class=\"area\" id=\"page-0-s1-a1\">")
        .nth(1)
        .unwrap()
        .split("</article>")
        .next()
        .unwrap();
    let area2 = html
        .split("<article class=\"area\" id=\"page-0-s1-a2\">")
        .nth(1)
        .unwrap()
        .split("</article>")
        .next()
        .unwrap();
    assert!(area1.contains("layer-2") && !area1.contains("layer-3"));
    assert!(area2.contains("layer-3") && !area2.contains("layer-2"));
    assert_eq!(html.matches("class=\"ln mod\"").count(), 3);
    let rest = html.split("Other changes in this screen").nth(1).unwrap();
    assert!(rest.contains("layer-4") && !rest.contains("layer-2"));
    assert!(html.contains("<div class=\"mini\" id=\"page-0-s1-a3\">"));
    assert_eq!(html.matches("class=\"cmp\" tabindex=\"0\"").count(), 3);
    // 2x pixels / 2 = natural CSS size; CSS only shrinks, never upscales
    assert!(html.contains("width=\"321\" height=\"160\""));
    assert!(html.contains(".cmp img{display:block;max-width:100%;height:auto;"));
    // one global switch: two radios open <body>, no per-page or per-card ids
    assert_eq!(html.matches("name=\"view\"").count(), 2);
    assert!(html.contains(
        "<body><input class=\"mr\" type=\"radio\" name=\"view\" id=\"view-side\" checked"
    ));
    assert!(html.contains("for=\"view-overlay\">Overlay</label>"));
    assert!(!html.contains("id=\"m-"));
    assert!(html.contains("#view-overlay:checked~.shell .cmp"));
    assert!(html.contains("@media print{body{background:#fff}.side,.tool,"));
    assert!(html.contains("style=\"left:5.00%;top:10.00%;width:15.00%;height:20.00%\""));
    assert!(html.contains("900 pixels changed"));
    assert!(html.contains("render/screen-after.png"));
    assert!(!html.contains("<details open"));
    assert!(html.contains("default-src 'none'; img-src 'self'; style-src 'unsafe-inline'"));
    assert!(!html.contains("<script"));
    assert!(html.contains("<b>Coverage</b><span>complete</span>"));
    assert!(!html.contains("Stored-change coverage:"));
    assert_eq!(html, render_html(&report));
}

#[test]
fn inherited_component_lines_stay_in_areas_and_unmatched_rows_count_in_footer() {
    let mut report = report();
    for node in &mut report.pages[0].changed {
        node.kind = "INSTANCE".into();
        node.changes = vec![figstudio_diff::PropertyChange {
            field: "derivedSymbolData.0.fillGeometry.0.styleID".into(),
            old: Some(figstudio_fig::Val::Int(1)),
            new: Some(figstudio_fig::Val::Int(2)),
            category: figstudio_diff::Category::Component,
        }];
    }
    let html = render_html(&report);
    assert_eq!(
        html.matches("shared component changed (see Changed areas)")
            .count(),
        2
    );
    // Nothing left for the page itself: no empty "Other changes on this page" section.
    assert!(!html.contains("Other changes on this page"));
    let rest = html.split("<p class=\"foot\">").nth(1).unwrap();
    assert!(!rest.contains("shared component changed"));
    assert!(!rest.contains("layer-4"));
    assert!(rest.contains("1 other property change on 1 layer is only in report.json"));
    for region in &mut report.pages[0].screens[0].regions {
        region.nodes.clear();
    }
    let html = render_html(&report);
    assert!(!html.contains("shared component changed"));
    assert!(
        html.contains("<section"),
        "a changed screen remains visible even without layer matches"
    );
}

#[test]
fn area_lines_deduplicate_membership_collapse_names_and_reject_unsafe_images() {
    let mut report = report();
    report.pages[0].changed[1].name = report.pages[0].changed[0].name.clone();
    let render = &mut report.pages[0].screens[0];
    render.regions[0].nodes = vec![Guid(0, 2), Guid(0, 2), Guid(0, 3)];
    render.regions[1].nodes.clear();
    render.regions[2].nodes.clear();
    let html = render_html(&report);
    assert_eq!(html.matches("class=\"ln mod\"").count(), 2);
    assert!(html.contains("class=\"ct\">×2</span>"));
    for path in [
        "javascript:alert(1)",
        "../private.png",
        "//host/x.png",
        "x%2f.png",
        "x\" onerror=\"bad",
    ] {
        let render = &mut report.pages[0].screens[0];
        render.thumb_new = Some(path.into());
        render.regions[0].old_png = Some(path.into());
        render.regions[0].new_png = Some(path.into());
        let html = render_html(&report);
        assert!(!html.contains(&format!("src=\"./{path}")));
        assert!(!html.contains("onerror=\"bad"));
    }
}

#[test]
fn before_after_labels_sit_above_the_picture_never_on_it() {
    let html = render_html(&report());
    let figures: Vec<_> = html
        .split("<figure class=\"")
        .skip(1)
        .filter(|f| f.starts_with("bf\"") || f.starts_with("af\""))
        .collect();
    assert!(figures.len() >= 6);
    for f in figures {
        let (caption, image) = (f.find("<figcaption>").unwrap(), f.find("<img").unwrap());
        assert!(caption < image && f[caption..image].contains("class=\"pill\">"));
    }
    // the label is flow content of the figure, not an overlay on the image
    let css = html
        .split("<style>")
        .nth(1)
        .unwrap()
        .split("</style>")
        .next()
        .unwrap();
    let rule = css
        .split(".cmp figcaption")
        .nth(1)
        .unwrap()
        .split('}')
        .next()
        .unwrap();
    assert!(!rule.contains("position:absolute"));
    // overlay: the opaque Before figure, caption included, covers After and shows on hover
    assert!(css.contains("#view-overlay:checked~.shell .cmp .bf{z-index:1;opacity:0"));
    assert!(css.contains("#view-overlay:checked~.shell .cmp:hover .bf"));
}

#[test]
fn an_area_without_crop_pictures_shows_no_before_after_control() {
    let mut report = report();
    for region in &mut report.pages[0].screens[0].regions {
        region.old_png = None;
        region.new_png = None;
    }
    let html = render_html(&report);
    assert!(!html.contains("class=\"cmp\" tabindex"), "{html}");
    assert!(!html.contains("aria-label=\"Area"), "{html}");
    // The area itself and where it is on the screen still show, its change lines take the
    // whole row, and the whole-screen pictures stand in for the missing crops.
    assert!(html.contains("class=\"loc\""));
    assert_eq!(
        html.matches("<article class=\"area bare\"").count(),
        2,
        "{html}"
    );
    assert!(html.contains(".area.bare .info{grid-column:1/-1}"));
    assert!(html.contains("class=\"cmp thumbs\""), "{html}");
    // One area folding into a shared card does not hide the screen pictures the others need.
    report.pages[0].screens[0].regions[2].shared = Some(0);
    report.shared_changes.push(figstudio_diff::SharedChange {
        id: 0,
        width: 30,
        height: 40,
        count: 2,
        old_png: Some("render/shared-before.png".into()),
        new_png: Some("render/shared-after.png".into()),
        occurrences: Vec::new(),
        component: None,
        component_view: false,
        image_width: Some(60),
        image_height: Some(80),
    });
    let html = render_html(&report);
    // The pair's own pictures, not the locator images on the area cards.
    let pair = html
        .split("<div class=\"cmp thumbs\"")
        .nth(1)
        .and_then(|rest| rest.split("class=\"hint\"").next())
        .unwrap_or_else(|| panic!("{html}"));
    assert!(
        pair.contains("aria-label=\"Screen: Before / After\""),
        "{html}"
    );
    assert!(pair.contains("src=\"./render/screen-after.png\""), "{html}");
    assert_eq!(
        html.matches("<article class=\"area bare\"").count(),
        2,
        "{html}"
    );
    // With the other areas cropped again, the shared one alone does not add the pair.
    for region in &mut report.pages[0].screens[0].regions[..2] {
        region.old_png = Some("render/zoom-before.png".into());
        region.new_png = Some("render/zoom-after.png".into());
    }
    let html = render_html(&report);
    assert!(html.contains("src=\"./render/zoom-after.png\""), "{html}");
    assert!(html.contains("src=\"./render/shared-after.png\""), "{html}");
    assert!(
        html.contains("aria-label=\"Area 1: Before / After\""),
        "{html}"
    );
    assert!(!html.contains("class=\"cmp thumbs\""), "{html}");
    assert!(
        !html.contains("aria-label=\"Screen: Before / After\""),
        "{html}"
    );
}

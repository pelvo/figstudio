use super::*;

fn changed(edit: impl Fn(&mut Val), kind: &str) -> String {
    let before = node(2, Some(1), kind, "c");
    let mut after = before.data.clone();
    edit(&mut after);
    let after = Node::from_data(after).unwrap();
    render_html(&diff_documents(
        &document(vec![before]),
        &document(vec![after]),
    ))
}
fn glyph(size: i64) -> Val {
    object(&[("fontSize", Val::Int(size)), ("commandsBlob", Val::Int(1))])
}

#[test]
fn derived_glyph_changes_become_one_component_content_line() {
    let html = changed(
        |d| {
            d.set(
                "derivedTextData",
                object(&[("glyphs", Val::Arr(vec![glyph(14)]))]),
            )
        },
        "INSTANCE",
    );
    let before = node(2, Some(1), "INSTANCE", "c");
    let mut b = before.data.clone();
    b.set(
        "derivedTextData",
        object(&[("glyphs", Val::Arr(vec![glyph(16), glyph(16)]))]),
    );
    let mut a = before.data.clone();
    a.set(
        "derivedTextData",
        object(&[("glyphs", Val::Arr(vec![glyph(14)]))]),
    );
    let report = diff_documents(
        &document(vec![Node::from_data(b).unwrap()]),
        &document(vec![Node::from_data(a).unwrap()]),
    );
    let html2 = render_html(&report);
    assert!(html.contains("component content updated"), "{html}");
    assert_eq!(html2.matches("class=\"ln mod\"").count(), 1, "{html2}");
    assert!(html2.contains("fontSize 16 → 14"), "{html2}");
    assert!(html2.contains("text changed"), "{html2}");
}
#[test]
fn ignored_derived_fields_make_no_line() {
    let html = changed(
        |d| {
            d.set(
                "derivedSymbolData",
                Val::Arr(vec![object(&[
                    ("commandsBlob", Val::Int(5)),
                    ("guidPath", Val::Str("x".into())),
                ])]),
            )
        },
        "INSTANCE",
    );
    assert!(!html.contains("<section"), "{html}");
}
#[test]
fn symbol_version_bump_is_component_updated() {
    let html = changed(
        |d| d.set("sharedSymbolVersion", Val::Str("v2".into())),
        "SYMBOL",
    );
    assert!(html.contains("component updated"), "{html}");
    assert!(html.contains("class=\"kd\">SYMBOL</span>"), "{html}");
}
fn with_region(mut report: DiffReport) -> DiffReport {
    let nodes = report.pages[0].changed.iter().map(|n| n.id).collect();
    let mut screen_report = diff_documents(
        &document(vec![node(9, Some(1), "FRAME", "")]),
        &document(vec![]),
    );
    let mut screen = screen_report.pages[0].screens.remove(0);
    screen.status = figstudio_diff::ScreenStatus::Changed;
    screen.pixels_changed = Some(900);
    screen.nodes = nodes;
    screen.regions = vec![Region {
        width: 3,
        height: 3,
        pixels_changed: 9,
        nodes: screen.nodes.clone(),
        ..Default::default()
    }];
    report.pages[0].screens.push(screen);
    report
}
fn derived_report(field: &str, old: Val, new: Val) -> DiffReport {
    let before = node(2, Some(1), "INSTANCE", "c");
    let mut b = before.data.clone();
    b.set("derivedSymbolData", object(&[(field, old)]));
    let mut a = before.data.clone();
    a.set("derivedSymbolData", object(&[(field, new)]));
    diff_documents(
        &document(vec![Node::from_data(b).unwrap()]),
        &document(vec![Node::from_data(a).unwrap()]),
    )
}
#[test]
fn inherited_instance_change_shows_page_only_with_regions() {
    let report = derived_report("styleID", Val::Int(1), Val::Int(2));
    let html = render_html(&with_region(report.clone()));
    assert!(html.contains("shared component changed"), "{html}");
    let html = render_html(&report);
    assert!(!html.contains("<section"), "{html}");
}
fn fonts(pairs: &[(&str, &str)]) -> Val {
    Val::Arr(
        pairs
            .iter()
            .map(|(k, v)| {
                object(&[
                    ("key", object(&[("family", Val::Str((*v).into()))])),
                    ("n", Val::Str((*k).into())),
                ])
            })
            .collect(),
    )
}
#[test]
fn font_family_index_shift_is_noise_but_real_change_shows() {
    let shift = |old: Val, new: Val| {
        let before = node(2, Some(1), "INSTANCE", "c");
        let mut b = before.data.clone();
        b.set("derivedTextData", object(&[("fontMetaData", old)]));
        let mut a = before.data.clone();
        a.set("derivedTextData", object(&[("fontMetaData", new)]));
        render_html(&diff_documents(
            &document(vec![Node::from_data(b).unwrap()]),
            &document(vec![Node::from_data(a).unwrap()]),
        ))
    };
    let font = |n: &str| object(&[("key", object(&[("family", Val::Str(n.into()))]))]);
    let html = shift(
        Val::Arr(vec![font("Noto"), object(&[])]),
        Val::Arr(vec![object(&[]), font("Noto")]),
    );
    assert!(!html.contains("family Noto"), "{html}");
    let html = shift(fonts(&[("a", "Noto")]), fonts(&[("a", "Plex")]));
    assert!(html.contains("family Noto → Plex"), "{html}");
}

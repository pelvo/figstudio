//! A sample compare report with each kind of screen. The test checks the page is
//! self-contained; with `FIGSTUDIO_SAMPLE_REPORT=<dir>` it also writes the report there for
//! the browser check in `web/e2e/compare-report.spec.mjs`.
mod support;
use figstudio_diff::{RenderOpts, diff_documents, render_html, render_report_to_dir};
use figstudio_fig::Document;
use support::{document, filled_rectangle_at, frame_at, named};

/// Old and new sides: a recoloured card, a nudged icon, a removed and an added screen, and
/// one unchanged screen moved on the canvas. All names are the test's own.
fn sample() -> (Document, Document) {
    let side = |new: bool| {
        let card = if new {
            (0.2, 0.4, 0.9)
        } else {
            (0.9, 0.2, 0.2)
        };
        let icon_x = if new { 26.0 } else { 20.0 };
        let moved_x = if new { 560.0 } else { 500.0 };
        let (gone, name) = if new {
            (7, "New settings")
        } else {
            (6, "Old settings")
        };
        let white = (1.0, 1.0, 1.0);
        document(vec![
            named(frame_at(2, 1, white, 0.0, 0.0, 390.0, 600.0), "Home"),
            named(
                filled_rectangle_at(3, 2, card, 20.0, 40.0, 350.0, 120.0),
                "Card",
            ),
            named(
                filled_rectangle_at(4, 2, (0.1, 0.1, 0.1), icon_x, 400.0, 24.0, 24.0),
                "Icon",
            ),
            named(frame_at(5, 1, white, moved_x, 0.0, 390.0, 600.0), "Moved"),
            named(
                frame_at(gone, 1, (0.95, 0.95, 0.95), 1000.0, 0.0, 390.0, 600.0),
                name,
            ),
        ])
    };
    (side(false), side(true))
}

#[test]
fn the_sample_report_is_self_contained_and_shows_each_kind_of_screen() {
    let (old, new) = sample();
    let mut report = diff_documents(&old, &new);
    let dir = std::env::var_os("FIGSTUDIO_SAMPLE_REPORT").map(std::path::PathBuf::from);
    let out = dir.clone().unwrap_or_else(|| {
        std::env::temp_dir().join(format!("figstudio-sample-report-{}", std::process::id()))
    });
    std::fs::create_dir_all(out.join("render")).unwrap();
    let none = |_: &figstudio_render::images::ImageRequest| None;
    render_report_to_dir(
        (&old, &new),
        &mut report,
        &RenderOpts::default(),
        (&none, &none),
        &out,
    )
    .unwrap();
    let html = render_html(&report);
    // Every picture the page links was written next to it.
    let pictures: Vec<_> = html
        .split("src=\"./")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    assert!(!pictures.is_empty(), "{html}");
    for path in &pictures {
        assert!(out.join(path).is_file(), "missing {path}");
    }
    if dir.is_some() {
        std::fs::write(out.join("report.html"), &html).unwrap();
    } else {
        let _ = std::fs::remove_dir_all(&out);
    }
    let page = &report.pages[0];
    let names: Vec<_> = page.screens.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Home", "Old settings", "New settings"], "{page:?}");
    assert_eq!((page.screens_unchanged, page.screens_moved), (1, 1));
    assert!(html.contains("Content-Security-Policy"));
    assert!(!html.contains("<script"));
    assert!(
        !html.contains("http://") && !html.contains("https://"),
        "{html}"
    );
    // The recolour has a change line; the nudged icon is a move, which shows only as an area.
    assert!(html.contains(">Card<"), "{html}");
    assert_eq!(page.screens[0].regions.len(), 2, "{:?}", page.screens[0]);
    // Each area's Before and After crop is on the page, not only the whole-page pictures.
    for region in &page.screens[0].regions {
        for path in [&region.old_png, &region.new_png] {
            let path = path.as_deref().unwrap_or_else(|| panic!("{region:?}"));
            assert!(pictures.contains(&path), "{path} not in the page");
        }
    }
}

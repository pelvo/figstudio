mod support;
use figstudio_diff::{MAX_REGIONS, RenderOpts, ScreenLimits, attach_render_diffs, diff_documents};
use figstudio_fig::{Node, Val};
use support::{document, filled_rectangle_at};

#[test]
fn screen_region_cap_does_not_undercount_changed_pixels_and_is_noted() {
    let side = |new| {
        let mut root = filled_rectangle_at(2, 1, (1.0, 1.0, 1.0), 0.0, 0.0, 1200.0, 300.0);
        root.data.set("type", Val::Str("FRAME".into()));
        document(
            std::iter::once(Node::from_data(root.data).unwrap())
                .chain((0..14).map(|i| {
                    filled_rectangle_at(
                        i + 3,
                        2,
                        if new {
                            (0.0, 1.0, 0.0)
                        } else {
                            (1.0, 0.0, 0.0)
                        },
                        20.0 + f64::from(i) * 80.0,
                        100.0,
                        10.0,
                        10.0,
                    )
                }))
                .collect(),
        )
    };
    let (old, new) = (side(false), side(true));
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    assert_eq!(report.pages[0].screens[0].regions.len(), MAX_REGIONS);
    assert_eq!(report.pages[0].screens[0].pixels_changed, Some(1400));
    assert!(report.truncated.iter().any(|n| n.contains("region limit")));
}

#[test]
fn page_crop_budget_is_shared_without_starving_later_screens() {
    let side = |color| {
        document(
            (2..6)
                .map(|id| {
                    let mut n = filled_rectangle_at(
                        id,
                        1,
                        color,
                        f64::from(id) * 1200.0,
                        0.0,
                        1000.0,
                        1000.0,
                    );
                    n.data.set("type", Val::Str("FRAME".into()));
                    Node::from_data(n.data).unwrap()
                })
                .collect(),
        )
    };
    let (old, new) = (side((1.0, 0.0, 0.0)), side((0.0, 1.0, 0.0)));
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    assert_eq!(report.pages[0].screens.len(), 4);
    for screen in &report.pages[0].screens {
        assert_eq!(screen.regions.len(), 1);
    }
    let mut total = 0;
    for screen in &images[&report.pages[0].id].screens {
        assert_eq!(screen.regions.len(), 1);
        for crop in &screen.regions {
            if !crop.after.is_empty() {
                let png = image::load_from_memory(&crop.after).unwrap();
                assert!(
                    png.width() > 1000 && png.height() > 1000,
                    "later crops must not be starved"
                );
                total += png.width() as usize * png.height() as usize * 2;
            }
        }
    }
    assert!(total <= figstudio_diff::MAX_RENDER_PIXELS);
    assert!(
        !images[&report.pages[0].id].screens.last().unwrap().regions[0]
            .after
            .is_empty()
    );
    let dir = std::env::temp_dir().join(format!("figstudio-crop-limit-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("render")).unwrap();
    let links =
        figstudio_diff::write_render_images(&dir, report.pages[0].id, &images[&report.pages[0].id])
            .unwrap();
    let page_id = report.pages[0].id;
    links.attach(&mut report, page_id);
    assert!(report.pages[0].screens[3].regions[0].new_png.is_some());
    assert!(image::open(dir.join("render/0-1-s3-a0-after.png")).is_ok());
    std::fs::remove_dir_all(dir).unwrap();
}

fn many_frames(count: u32, side: f64, new: bool) -> figstudio_fig::Document {
    frames(count, side, |_| new)
}

fn frames(count: u32, side: f64, new: impl Fn(u32) -> bool) -> figstudio_fig::Document {
    document(
        (0..count)
            .flat_map(|i| {
                let root = 10 + i * 2;
                let mut frame = filled_rectangle_at(
                    root,
                    1,
                    (1.0, 1.0, 1.0),
                    f64::from(i) * (side + 20.0),
                    0.0,
                    side,
                    side.min(1000.0),
                );
                frame.data.set("type", Val::Str("FRAME".into()));
                let dot = if new(i) {
                    (0.0, 1.0, 0.0)
                } else {
                    (1.0, 0.0, 0.0)
                };
                [
                    Node::from_data(frame.data).unwrap(),
                    filled_rectangle_at(root + 1, root, dot, 10.0, 10.0, 20.0, 20.0),
                ]
            })
            .collect(),
    )
}

fn skipped_screens(report: &figstudio_diff::DiffReport) -> usize {
    report.pages[0]
        .screens
        .iter()
        .filter(|s| s.pixels_changed.is_none())
        .count()
}

fn limited(limits: ScreenLimits) -> RenderOpts {
    RenderOpts {
        screen_limits: limits,
        ..RenderOpts::default()
    }
}

fn run(count: u32, side: f64, opts: &RenderOpts) -> figstudio_diff::DiffReport {
    let (old, new) = (
        many_frames(count, side, false),
        many_frames(count, side, true),
    );
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, opts);
    assert_eq!(report.pages[0].screens.len(), count as usize);
    assert_eq!(images[&report.pages[0].id].screens.len(), count as usize);
    report
}

fn has_note(report: &figstudio_diff::DiffReport, skipped: usize) -> bool {
    let id = report.pages[0].id;
    report.truncated.iter().any(|n| {
        n.contains("screen limit reached")
            && n.contains(&format!("page {id}"))
            && n.contains(&format!("{skipped} screens were not rendered"))
    })
}

#[test]
fn a_page_with_too_many_screens_stops_rendering_and_says_so() {
    let limits = ScreenLimits {
        page_screens: 5,
        ..ScreenLimits::default()
    };
    let report = run(9, 60.0, &limited(limits));
    assert_eq!(skipped_screens(&report), 4);
    assert!(has_note(&report, 4));
}

#[test]
fn the_document_pixel_budget_stops_standalone_screen_rendering() {
    // Each 4000x1000 screen costs 8 MP (both sides); the budget holds exactly 2.
    let limits = ScreenLimits {
        document_pixels: 16_000_000,
        ..ScreenLimits::default()
    };
    let report = run(5, 4000.0, &limited(limits));
    assert_eq!(skipped_screens(&report), 3);
    assert!(has_note(&report, 3));
}

/// Encoded bytes of the three whole-page pictures of a comparison, which the cap counts too.
fn page_picture_bytes(old: &figstudio_fig::Document, new: &figstudio_fig::Document) -> usize {
    let mut report = diff_documents(old, new);
    let images = attach_render_diffs(old, new, &mut report, &RenderOpts::default());
    let page = &images[&report.pages[0].id];
    [&page.before, &page.after, &page.diff]
        .into_iter()
        .map(|p| p.as_ref().map_or(0, Vec::len))
        .sum()
}

#[test]
fn the_retained_picture_byte_cap_stops_standalone_screen_rendering() {
    let (old, new) = (many_frames(4, 60.0, false), many_frames(4, 60.0, true));
    // Room for the page pictures plus one byte: the first screen is drawn and pushes the
    // page over the cap, so the rest are skipped.
    let limits = ScreenLimits {
        page_bytes: page_picture_bytes(&old, &new) + 1,
        ..ScreenLimits::default()
    };
    let report = run(4, 60.0, &limited(limits));
    assert_eq!(skipped_screens(&report), 3);
    assert!(has_note(&report, 3));
}

#[test]
fn whole_page_pictures_count_toward_the_page_byte_cap() {
    let (old, new) = (many_frames(4, 60.0, false), many_frames(4, 60.0, true));
    // The page pictures alone fill the cap, so no screen is drawn.
    let limits = ScreenLimits {
        page_bytes: page_picture_bytes(&old, &new),
        ..ScreenLimits::default()
    };
    let report = run(4, 60.0, &limited(limits));
    assert_eq!(skipped_screens(&report), 4);
    assert!(has_note(&report, 4));
}

#[test]
fn many_medium_screens_under_the_default_caps_all_render() {
    // 12 screens of 8 MP (both sides) is 96 MP, 300 small ones is past the old 256 count.
    for (count, side) in [(12, 4000.0), (300, 60.0)] {
        let (old, new) = (
            many_frames(count, side, false),
            many_frames(count, side, true),
        );
        let mut report = diff_documents(&old, &new);
        let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
        assert_eq!(report.pages[0].screens.len(), count as usize);
        assert_eq!(skipped_screens(&report), 0);
        assert!(report.truncated.iter().all(|n| !n.contains("screen limit")));
    }
}

#[test]
fn dropped_unchanged_screens_do_not_consume_the_page_byte_cap() {
    // Four identical screens are dropped from the report; the changed one after them must draw.
    let (old, new) = (frames(5, 60.0, |_| false), frames(5, 60.0, |i| i == 4));
    let limits = ScreenLimits {
        page_bytes: page_picture_bytes(&old, &new) + 1,
        ..ScreenLimits::default()
    };
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &limited(limits));
    let page = &report.pages[0];
    assert_eq!(page.screens_unchanged, 4);
    assert_eq!(page.screens.len(), 1);
    assert_eq!(page.screens[0].pixels_changed, Some(400));
    assert!(images[&page.id].screens[0].before.is_some());
    assert!(report.truncated.iter().all(|n| !n.contains("screen limit")));
}

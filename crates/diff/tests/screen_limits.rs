mod support;
use figstudio_diff::{MAX_REGIONS, RenderOpts, ScreenLimits, attach_render_diffs, diff_documents};
use figstudio_fig::{Node, Val};
use support::{document, filled_rectangle_at, object};

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
    // Every dot moves (no change line, so all five screens share one draw tier and keep id
    // order), but the first four dots are hidden: those screens draw identically and are
    // dropped. The changed fifth screen comes after them and must still draw.
    let hide = |doc: &mut figstudio_fig::Document, moved: bool| {
        let mut nodes = doc.nodes.clone();
        for (i, dot) in [11, 13, 15, 17, 19].into_iter().enumerate() {
            let n = nodes
                .iter_mut()
                .find(|n| n.id == figstudio_fig::Guid(0, dot))
                .unwrap();
            if i < 4 {
                n.data.set("visible", Val::Bool(false));
            }
            if moved {
                n.data.set(
                    "transform",
                    support::object(&[("m02", Val::Float(30.0)), ("m12", Val::Float(10.0))]),
                );
            }
            *n = Node::from_data(n.data.clone()).unwrap();
        }
        doc.set_nodes(nodes).unwrap();
    };
    let mut old = frames(5, 60.0, |_| false);
    hide(&mut old, false);
    let mut new = frames(5, 60.0, |_| false);
    hide(&mut new, true);
    let limits = ScreenLimits {
        page_bytes: page_picture_bytes(&old, &new) + 1,
        ..ScreenLimits::default()
    };
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &limited(limits));
    let page = &report.pages[0];
    assert_eq!(page.screens_unchanged, 4);
    assert_eq!(page.screens.len(), 1);
    assert_eq!(page.screens[0].pixels_changed, Some(800));
    assert!(images[&page.id].screens[0].before.is_some());
    assert!(report.truncated.iter().all(|n| !n.contains("screen limit")));
}

#[test]
fn unchanged_screens_do_not_use_up_the_screen_cap_before_changed_ones() {
    // Screen 2 is unchanged and comes first in id order; screen 3's child is recoloured.
    let side = |rgb| {
        let frame = |id, x| {
            let mut n = filled_rectangle_at(id, 1, (1.0, 1.0, 1.0), x, 0.0, 200.0, 200.0);
            n.data.set("type", Val::Str("FRAME".into()));
            Node::from_data(n.data).unwrap()
        };
        document(vec![
            frame(2, 0.0),
            frame(3, 400.0),
            filled_rectangle_at(4, 3, rgb, 20.0, 20.0, 40.0, 40.0),
        ])
    };
    let (old, new) = (side((1.0, 0.0, 0.0)), side((0.0, 1.0, 0.0)));
    let mut report = diff_documents(&old, &new);
    let opts = RenderOpts {
        screen_limits: ScreenLimits {
            page_screens: 1,
            ..ScreenLimits::default()
        },
        ..RenderOpts::default()
    };
    let images = attach_render_diffs(&old, &new, &mut report, &opts);
    let page = &report.pages[0];
    assert_eq!(page.screens.len(), 1, "{page:?}");
    assert_eq!(page.screens[0].new_id, Some(figstudio_fig::Guid(0, 3)));
    assert_eq!(page.screens[0].regions.len(), 1);
    assert!(images[&page.id].screens[0].after.is_some());
}

#[test]
fn added_and_removed_screens_do_not_take_a_crop_share() {
    // One changed 1000×1000 screen next to many added screens: it gets the whole budget.
    let frame = |id, x, rgb| {
        let mut n = filled_rectangle_at(id, 1, rgb, x, 0.0, 1000.0, 1000.0);
        n.data.set("type", Val::Str("FRAME".into()));
        Node::from_data(n.data).unwrap()
    };
    let old = document(vec![frame(2, 0.0, (1.0, 0.0, 0.0))]);
    let new = document(
        std::iter::once(frame(2, 0.0, (0.0, 1.0, 0.0)))
            .chain((3..40).map(|id| frame(id, f64::from(id) * 1200.0, (0.0, 0.0, 1.0))))
            .collect(),
    );
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let page = &report.pages[0];
    let at = page
        .screens
        .iter()
        .position(|s| s.new_id == Some(figstudio_fig::Guid(0, 2)))
        .unwrap();
    let crop = &images[&page.id].screens[at].regions[0];
    let png = image::load_from_memory(&crop.after).unwrap();
    assert!(
        png.width() >= 2000 && png.height() >= 2000,
        "{}x{}",
        png.width(),
        png.height()
    );
}

#[test]
fn unchanged_screens_do_not_dilute_a_changed_screens_crop_share() {
    // Ten unchanged 1000×1000 screens and one changed one: the changed crop gets the whole
    // budget (2x, so at least 2000 px a side), not a tenth of it.
    let frame = |id, x, rgb| {
        let mut n = filled_rectangle_at(id, 1, rgb, x, 0.0, 1000.0, 1000.0);
        n.data.set("type", Val::Str("FRAME".into()));
        Node::from_data(n.data).unwrap()
    };
    let side = |rgb| {
        document(
            (2..12)
                .map(|id| frame(id, f64::from(id) * 1200.0, (0.0, 0.0, 1.0)))
                .chain(std::iter::once(frame(20, 0.0, rgb)))
                .collect(),
        )
    };
    let (old, new) = (side((1.0, 0.0, 0.0)), side((0.0, 1.0, 0.0)));
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let page = &report.pages[0];
    assert_eq!(page.screens.len(), 1);
    // The ten frames were matched and dropped, so they were in the running for the budget.
    assert_eq!(page.screens_unchanged, 10);
    let crop = &images[&page.id].screens[0].regions[0];
    let png = image::load_from_memory(&crop.after).unwrap();
    assert!(
        png.width() >= 2000 && png.height() >= 2000,
        "{}x{}",
        png.width(),
        png.height()
    );
}

#[test]
fn canvas_moves_do_not_dilute_an_inner_moves_crop_share() {
    // Screen 2's child moves (no change line, real pixels); ten later frames only move on the
    // canvas, which does not change their own pictures. The inner move keeps the full budget.
    let frame = |id, x| {
        let mut n = filled_rectangle_at(id, 1, (1.0, 1.0, 1.0), x, 0.0, 1000.0, 1000.0);
        n.data.set("type", Val::Str("FRAME".into()));
        Node::from_data(n.data).unwrap()
    };
    let side = |new: bool| {
        let shift = if new { 50.0 } else { 0.0 };
        document(
            std::iter::once(frame(2, 0.0))
                .chain(std::iter::once(filled_rectangle_at(
                    3,
                    2,
                    (1.0, 0.0, 0.0),
                    100.0 + shift * 10.0,
                    100.0,
                    600.0,
                    600.0,
                )))
                .chain((10..20).map(|id| frame(id, f64::from(id) * 1200.0 + shift)))
                .collect(),
        )
    };
    let (old, new) = (side(false), side(true));
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let page = &report.pages[0];
    assert_eq!(page.screens.len(), 1, "{page:?}");
    assert_eq!((page.screens_unchanged, page.screens_moved), (10, 10));
    // Not scaled down: the crop is drawn at 2x its area.
    let region = &page.screens[0].regions[0];
    let crop = &images[&page.id].screens[0].regions[0];
    let png = image::load_from_memory(&crop.after).unwrap();
    assert!(
        png.width() >= 2 * region.width && png.height() >= 2 * region.height,
        "{}x{} for {}x{}",
        png.width(),
        png.height(),
        region.width,
        region.height
    );
}

#[test]
fn a_resized_root_without_a_change_line_is_not_diluted_by_unchanged_screens() {
    // Screen 2 is resized (a root edit with no change line, but new pixels); ten frames that
    // are unchanged, only renamed, or only flipped follow it. A screen is drawn in its own
    // frame, so a flip cannot change it. The resize keeps the full crop budget.
    let frame = |id, x, w, name: &str, flipped: bool| {
        let mut n = filled_rectangle_at(id, 1, (1.0, 0.0, 0.0), x, 0.0, w, 1000.0);
        n.data.set("type", Val::Str("FRAME".into()));
        n.data.set("name", Val::Str(name.into()));
        if flipped {
            let matrix = [("m00", -1.0), ("m02", x + w), ("m12", 0.0)];
            let matrix: Vec<_> = matrix.iter().map(|&(k, v)| (k, Val::Float(v))).collect();
            n.data.set("transform", object(&matrix));
        }
        Node::from_data(n.data).unwrap()
    };
    let side = |w, name: &str, flipped| {
        document(
            std::iter::once(frame(2, 0.0, w, "Resized", false))
                .chain((10..20).map(|id| frame(id, f64::from(id) * 1200.0, 1000.0, name, flipped)))
                .collect(),
        )
    };
    for (renamed, flipped) in [(false, false), (true, false), (false, true)] {
        let old = side(400.0, "Frame", false);
        let new = side(1000.0, if renamed { "Renamed" } else { "Frame" }, flipped);
        let mut report = diff_documents(&old, &new);
        let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
        let page = &report.pages[0];
        assert_eq!(
            page.screens.len(),
            1,
            "renamed {renamed}, flipped {flipped}: {page:?}"
        );
        assert_eq!(page.screens_unchanged, 10);
        let region = &page.screens[0].regions[0];
        let crop = &images[&page.id].screens[0].regions[0];
        let png = image::load_from_memory(&crop.after).unwrap();
        assert!(
            png.width() >= 2 * region.width && png.height() >= 2 * region.height,
            "renamed {renamed}, flipped {flipped}: {}x{} for {}x{}",
            png.width(),
            png.height(),
            region.width,
            region.height
        );
    }
}

#[test]
fn edits_that_draw_nothing_do_not_dilute_an_earlier_real_change() {
    // Screen 2 comes first and its child moves (real pixels, no change line). Ten later
    // screens each have a child that is only renamed, so their pictures stay the same. Only
    // screens with changed areas share the crop budget, so the move keeps all of it.
    let frame = |id, x| {
        let mut n = filled_rectangle_at(id, 1, (1.0, 1.0, 1.0), x, 0.0, 1000.0, 1000.0);
        n.data.set("type", Val::Str("FRAME".into()));
        Node::from_data(n.data).unwrap()
    };
    let side = |new: bool| {
        let child = |id, parent, x, name: &str| {
            let mut n = filled_rectangle_at(id, parent, (1.0, 0.0, 0.0), x, 100.0, 600.0, 600.0);
            n.data.set("name", Val::Str(name.into()));
            Node::from_data(n.data).unwrap()
        };
        let mut nodes = vec![
            frame(2, 0.0),
            child(3, 2, if new { 600.0 } else { 100.0 }, "Moved"),
        ];
        for id in 10..20 {
            nodes.push(frame(id, f64::from(id) * 1200.0));
            nodes.push(child(
                id + 100,
                id,
                100.0,
                if new { "Renamed" } else { "Child" },
            ));
        }
        document(nodes)
    };
    let (old, new) = (side(false), side(true));
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let page = &report.pages[0];
    assert_eq!(page.screens.len(), 1, "{page:?}");
    assert_eq!(page.screens_unchanged, 10);
    let region = &page.screens[0].regions[0];
    let crop = &images[&page.id].screens[0].regions[0];
    let png = image::load_from_memory(&crop.after).unwrap();
    assert!(
        png.width() >= 2 * region.width && png.height() >= 2 * region.height,
        "{}x{} for {}x{}",
        png.width(),
        png.height(),
        region.width,
        region.height
    );
}

#[test]
fn a_renamed_or_moved_root_does_not_use_up_the_screen_cap_before_a_real_change() {
    // Screen 2 comes first and is only renamed or moved on the canvas, which cannot change
    // its picture; screen 3's child moves (real pixels, no change line). With room for one
    // screen, screen 3 is the one drawn.
    let frame = |id, x, name: &str| {
        let mut n = filled_rectangle_at(id, 1, (1.0, 1.0, 1.0), x, 0.0, 200.0, 200.0);
        n.data.set("type", Val::Str("FRAME".into()));
        n.data.set("name", Val::Str(name.into()));
        Node::from_data(n.data).unwrap()
    };
    for edit in ["rename", "canvas move"] {
        let side = |new: bool| {
            let (name, x) = match (new, edit) {
                (true, "rename") => ("Renamed", 0.0),
                (true, _) => ("Frame", -300.0),
                _ => ("Frame", 0.0),
            };
            let child_x = if new { 80.0 } else { 20.0 };
            document(vec![
                frame(2, x, name),
                frame(3, 400.0, "Other"),
                filled_rectangle_at(4, 3, (1.0, 0.0, 0.0), child_x, 20.0, 40.0, 40.0),
            ])
        };
        let (old, new) = (side(false), side(true));
        let mut report = diff_documents(&old, &new);
        let opts = RenderOpts {
            screen_limits: ScreenLimits {
                page_screens: 1,
                ..ScreenLimits::default()
            },
            ..RenderOpts::default()
        };
        let images = attach_render_diffs(&old, &new, &mut report, &opts);
        let page = &report.pages[0];
        let at = page
            .screens
            .iter()
            .position(|s| s.new_id == Some(figstudio_fig::Guid(0, 3)))
            .unwrap_or_else(|| panic!("{edit}: {page:?}"));
        assert_eq!(page.screens[at].regions.len(), 1, "{edit}");
        assert!(images[&page.id].screens[at].after.is_some(), "{edit}");
    }
}

#[test]
fn crop_pictures_count_toward_the_page_byte_cap() {
    // Room for the page pictures and every whole-screen picture, but not for every crop:
    // once the first screen's crops fill the cap, later screens cut none.
    let (old, new) = (many_frames(4, 60.0, false), many_frames(4, 60.0, true));
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let screens = &images[&report.pages[0].id].screens;
    let len = |v: &Option<Vec<u8>>| v.as_ref().map_or(0, Vec::len);
    let whole: usize = screens.iter().map(|s| len(&s.before) + len(&s.after)).sum();
    assert!(screens.iter().all(|s| !s.regions.is_empty()));
    let limits = ScreenLimits {
        page_bytes: page_picture_bytes(&old, &new) + whole + 1,
        ..ScreenLimits::default()
    };
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &limited(limits));
    let screens = &images[&report.pages[0].id].screens;
    assert_eq!(screens.len(), 4);
    let cropped = screens.iter().filter(|s| !s.regions.is_empty()).count();
    assert_eq!(cropped, 1);
    // Screens past the cap still list their areas and the layers in them.
    let page = &report.pages[0];
    assert!(
        page.screens
            .iter()
            .all(|s| s.regions.len() == 1 && !s.regions[0].nodes.is_empty())
    );
    let note = |report: &figstudio_diff::DiffReport, screens: &str| {
        let note = format!(
            "page {} picture byte limit reached; {screens} without crops",
            report.pages[0].id
        );
        assert!(report.truncated.contains(&note), "{:?}", report.truncated);
    };
    note(&report, "3 screens show their areas");
    // When the first screen's own pictures fill the cap, it keeps its area too.
    let limits = ScreenLimits {
        page_bytes: page_picture_bytes(&old, &new) + 1,
        ..ScreenLimits::default()
    };
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &limited(limits));
    let drawn = report.pages[0]
        .screens
        .iter()
        .position(|s| s.pixels_changed.is_some())
        .unwrap();
    assert_eq!(report.pages[0].screens[drawn].regions.len(), 1);
    assert!(
        images[&report.pages[0].id].screens[drawn]
            .regions
            .is_empty()
    );
    // The screens past it were not rendered at all, so the note names only that one.
    note(&report, "1 screen shows its areas");
}

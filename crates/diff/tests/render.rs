mod support;

use figstudio_diff::{
    MAX_RENDER_PIXELS, PageStatus, RenderOpts, attach_render_diffs, diff_documents,
    render_page_pair,
};
use figstudio_fig::{Guid, Node, Val};
use support::{document, filled_rectangle, filled_rectangle_at, object, two_page_document};

const RED: (f64, f64, f64) = (1.0, 0.0, 0.0);
const GREEN: (f64, f64, f64) = (0.0, 1.0, 0.0);

fn filled_frame(rgb: (f64, f64, f64)) -> Node {
    let mut n = filled_rectangle(2, 1, rgb);
    n.data.set("type", Val::Str("FRAME".into()));
    Node::from_data(n.data).unwrap()
}

#[test]
fn recolor_changes_only_that_page() {
    let old = two_page_document(vec![filled_frame(RED)], vec![filled_rectangle(12, 11, RED)]);
    let new = two_page_document(
        vec![filled_frame(GREEN)],
        vec![filled_rectangle(12, 11, RED)],
    );
    let mut report = diff_documents(&old, &new);
    assert_eq!(report.pages.len(), 1, "only the recolored page is listed");
    let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let page = &report.pages[0];
    assert_eq!(page.id, Guid(0, 1));
    let render = page.render.as_ref().expect("changed page has a render");
    assert!(render.old_png.is_none() && render.new_png.is_none() && render.diff_png.is_none());
    let count = render.pixels_changed.expect("changed pixels counted");
    // The rect covers 40x40=1600 px; pixel-aligned edges cover them fully.
    assert!(count > 0 && count <= 40 * 40, "count {count}");
    let (id, entry) = images.iter().next().expect("images returned");
    assert_eq!(*id, Guid(0, 1));
    assert!(entry.before.is_some() && entry.after.is_some() && entry.diff.is_some());
    assert_eq!(report.truncated, Vec::<String>::new());
    let screen = &page.screens[0];
    assert_eq!(screen.regions.len(), 1, "one changed blob, one area");
    assert_eq!(screen.regions[0].pixels_changed, count);
    assert_eq!(entry.screens[0].regions.len(), 1);
    let (w, h) = (screen.regions[0].width, screen.regions[0].height);
    let crop = image::load_from_memory(&entry.screens[0].regions[0].after).unwrap();
    assert_eq!((crop.width(), crop.height()), (w * 2, h * 2));
}

#[test]
fn unchanged_documents_get_no_render_entries() {
    let old = document(vec![filled_rectangle(2, 1, RED)]);
    let new = document(vec![filled_rectangle(2, 1, RED)]);
    let mut report = diff_documents(&old, &new);
    assert!(report.pages.is_empty());
    let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    assert!(images.is_empty());
    // A pair rendered directly still aligns and shows no change.
    let (render, images) = render_page_pair(
        &old,
        &new,
        Some(Guid(0, 1)),
        Some(Guid(0, 1)),
        &RenderOpts::default(),
    )
    .expect("page renders");
    assert_eq!(render.pixels_changed, Some(0));
    assert!(images.before.is_some() && images.after.is_some());
}

#[test]
fn added_page_counts_present_side_ink() {
    let old = document(vec![]);
    let new = two_page_document(vec![], vec![filled_rectangle(12, 11, RED)]);
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let page = &report.pages[0];
    assert_eq!(page.status, PageStatus::Added);
    let render = page.render.as_ref().expect("added page has a render");
    let count = render.pixels_changed.expect("ink counted");
    assert!(count > 0 && count <= 40 * 40, "count {count}");
    let entry = &images[&Guid(0, 11)];
    assert!(entry.before.is_none() && entry.after.is_some() && entry.diff.is_some());
    // Only the new side was rendered, so its bounds drive the canvas
    // (content bounds can include a few pixels of ink overflow).
    assert!(render.width >= Some(40) && render.height >= Some(40));
}

#[test]
fn removed_page_counts_present_side_ink() {
    let old = two_page_document(vec![], vec![filled_rectangle(12, 11, RED)]);
    let new = document(vec![]);
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let page = &report.pages[0];
    assert_eq!(page.status, PageStatus::Removed);
    let count = page
        .render
        .as_ref()
        .and_then(|render| render.pixels_changed)
        .expect("ink counted");
    assert!(count > 0 && count <= 40 * 40, "count {count}");
    let entry = &images[&Guid(0, 11)];
    assert!(entry.before.is_some() && entry.after.is_none() && entry.diff.is_some());
}

#[test]
fn oversized_page_is_scaled_down_and_noted() {
    // 6000x3000 at scale 1 is 18M px, over the 16M budget.
    let mut rect = filled_frame(RED);
    rect.data.set(
        "size",
        support::object(&[
            ("x", figstudio_fig::Val::Float(6000.0)),
            ("y", figstudio_fig::Val::Float(3000.0)),
        ]),
    );
    let rect = figstudio_fig::Node::from_data(rect.data).unwrap();
    let old = document(vec![rect.clone()]);
    let mut green = rect.clone();
    green.data.set("fillPaints", support::solid(0.0, 1.0, 0.0));
    let new = document(vec![figstudio_fig::Node::from_data(green.data).unwrap()]);
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let crops = &images[&Guid(0, 1)].screens[0].regions;
    let crop = image::load_from_memory(&crops[0].after).unwrap();
    assert_eq!(crop.width().max(crop.height()), 2048);
    let crop_pixels: u64 = crops
        .iter()
        .map(|c| {
            let img = image::load_from_memory(&c.after).unwrap();
            u64::from(img.width()) * u64::from(img.height()) * 2
        })
        .sum();
    assert!(crop_pixels <= MAX_RENDER_PIXELS as u64);
    assert!(
        report
            .truncated
            .iter()
            .any(|n| n.contains("crop") && n.contains("capped"))
    );
    let render = report.pages[0].render.as_ref().expect("render attached");
    let pixels = u64::from(render.width.unwrap()) * u64::from(render.height.unwrap());
    assert!(
        pixels <= u64::try_from(MAX_RENDER_PIXELS).unwrap(),
        "{pixels}"
    );
    assert!(render.pixels_changed.unwrap() > 0);
    assert!(
        report
            .truncated
            .iter()
            .any(|note| note.contains("pixel budget")),
        "{:?}",
        report.truncated
    );
}

#[test]
fn tall_over_budget_page_keeps_uniform_scale() {
    // 1025x16380 at scale 1 is over the 16M budget; ceil can still overflow, and the
    // old crop cut height only — a recolor at the bottom then vanished.
    let top = filled_rectangle_at(2, 1, RED, 0.0, 0.0, 1025.0, 1.0);
    let old_bottom = filled_rectangle_at(3, 1, RED, 0.0, 16_378.0, 1025.0, 2.0);
    let new_bottom = filled_rectangle_at(3, 1, GREEN, 0.0, 16_378.0, 1025.0, 2.0);
    let old = document(vec![top.clone(), old_bottom]);
    let new = document(vec![top, new_bottom]);
    let (render, _) = render_page_pair(
        &old,
        &new,
        Some(Guid(0, 1)),
        Some(Guid(0, 1)),
        &RenderOpts::default(),
    )
    .expect("page renders");
    let width = u64::from(render.width.unwrap());
    let height = u64::from(render.height.unwrap());
    assert!(width * height <= u64::try_from(MAX_RENDER_PIXELS).unwrap());
    assert!(render.pixels_changed.unwrap() > 0);
}

#[test]
fn max_side_clamp_is_noted() {
    // 100004x14 at scale 1 is under the pixel budget but over vello's max side.
    let rect = filled_rectangle_at(2, 1, RED, 0.0, 0.0, 100_004.0, 14.0);
    let old = document(vec![rect.clone()]);
    let mut green = rect.clone();
    green.data.set("fillPaints", support::solid(0.0, 1.0, 0.0));
    let new = document(vec![Node::from_data(green.data).unwrap()]);
    let mut report = diff_documents(&old, &new);
    let _images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let render = report.pages[0].render.as_ref().expect("render attached");
    assert!(render.width.unwrap() <= figstudio_render::MAX_SIDE);
    assert!(render.height.unwrap() <= figstudio_render::MAX_SIDE);
    assert!(
        report
            .truncated
            .iter()
            .any(|note| note.contains("max side")),
        "{:?}",
        report.truncated
    );
}

#[test]
fn tiny_scale_added_page_still_counts_ink() {
    let old = document(vec![]);
    let new = document(vec![filled_rectangle(2, 1, RED)]);
    let (render, _) = render_page_pair(
        &old,
        &new,
        None,
        Some(Guid(0, 1)),
        &RenderOpts {
            scale: 0.01,
            ..RenderOpts::default()
        },
    )
    .expect("page renders");
    assert_eq!(render.width, Some(1));
    assert_eq!(render.height, Some(1));
    assert!(render.pixels_changed.unwrap() > 0);
}

#[test]
fn translucent_page_png_is_straight_alpha() {
    let mut canvas_bg = document(vec![filled_rectangle(2, 1, RED)]);
    let mut nodes = vec![
        support::node(0, None, "DOCUMENT", ""),
        support::node(1, Some(0), "CANVAS", "a"),
        filled_rectangle(2, 1, RED),
    ];
    nodes[1].data.set(
        "backgroundColor",
        object(&[
            ("r", Val::Float(1.0)),
            ("g", Val::Float(0.0)),
            ("b", Val::Float(0.0)),
            ("a", Val::Float(0.5)),
        ]),
    );
    canvas_bg.set_nodes(nodes).unwrap();
    let (render, images) = render_page_pair(
        &canvas_bg,
        &canvas_bg,
        Some(Guid(0, 1)),
        Some(Guid(0, 1)),
        &RenderOpts::default(),
    )
    .expect("page renders");
    assert_eq!(render.pixels_changed, Some(0));
    let png = images.before.expect("before png");
    let decoded = image::load_from_memory(&png).unwrap().into_rgba8();
    let corner = decoded.get_pixel(0, 0).0;
    // Premul 0.5 red written as straight alpha is ~[255, 0, 0, 128], not ~[128, 0, 0, 128].
    assert!(corner[0] > 200, "{corner:?}");
    assert!(corner[1] < 20 && corner[2] < 20, "{corner:?}");
    assert!((i16::from(corner[3]) - 128).abs() <= 8, "{corner:?}");
}

#[test]
fn recolor_records_worst_channel_delta() {
    let old = document(vec![filled_rectangle(2, 1, RED)]);
    let new = document(vec![filled_rectangle(2, 1, GREEN)]);
    let (render, _) = render_page_pair(
        &old,
        &new,
        Some(Guid(0, 1)),
        Some(Guid(0, 1)),
        &RenderOpts::default(),
    )
    .expect("page renders");
    assert_eq!(render.worst_channel_delta, Some(255));
}

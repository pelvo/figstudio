//! A small visible edit (a nudged icon, one changed character) is never dropped as noise
//! when it is the only thing that changed on a screen.
mod support;
use figstudio_diff::{RenderOpts, ScreenStatus, attach_render_diffs, diff_documents, render_html};
use figstudio_fig::{Node, Val};
use support::{document, filled_rectangle_at, frame_at, named};

fn screen() -> Node {
    named(
        frame_at(2, 1, (1.0, 1.0, 1.0), 0.0, 0.0, 300.0, 200.0),
        "Screen",
    )
}

#[test]
fn a_small_moved_layer_keeps_its_screen_and_area() {
    // A 4×4 dot nudged 10 px: 32 changed pixels, below the speck floor, and a position-only
    // edit has no change line. It used to vanish: screen counted as unchanged, report empty.
    let side = |x| {
        document(vec![
            screen(),
            filled_rectangle_at(3, 2, (1.0, 0.0, 0.0), x, 50.0, 4.0, 4.0),
        ])
    };
    let (old, new) = (side(100.0), side(110.0));
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let page = &report.pages[0];
    assert_eq!(page.screens_unchanged, 0, "{page:?}");
    let s = &page.screens[0];
    assert_eq!(s.status, ScreenStatus::Changed);
    assert_eq!(s.pixels_changed, Some(32));
    assert_eq!(s.regions.len(), 1, "{:?}", s.regions);
    assert_eq!(s.regions[0].pixels_changed, 32);
    let html = render_html(&report);
    assert!(html.contains("id=\"page-0-s1-a1\""), "{html}");
}

#[test]
fn a_speck_next_to_a_real_change_is_still_noise_but_counted() {
    let side = |new: bool| {
        let (big, dot) = if new {
            ((0.0, 1.0, 0.0), 110.0)
        } else {
            ((1.0, 0.0, 0.0), 100.0)
        };
        document(vec![
            screen(),
            filled_rectangle_at(3, 2, big, 10.0, 10.0, 20.0, 20.0),
            filled_rectangle_at(4, 2, (0.0, 0.0, 1.0), dot, 150.0, 4.0, 4.0),
        ])
    };
    let (old, new) = (side(false), side(true));
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let s = &report.pages[0].screens[0];
    assert_eq!(s.regions.len(), 1, "{:?}", s.regions);
    assert_eq!(s.regions[0].pixels_changed, 400);
    // The screen total is every changed pixel, not only the kept areas.
    assert_eq!(s.pixels_changed, Some(432));
}

#[test]
fn a_changed_screen_with_no_visible_pixels_says_so_and_shows_the_screen() {
    // The recoloured layer sits outside the frame, which clips it: an edit with no pixels.
    let side = |rgb| {
        document(vec![
            screen(),
            filled_rectangle_at(3, 2, rgb, 1000.0, 50.0, 20.0, 20.0),
        ])
    };
    let (old, new) = (side((1.0, 0.0, 0.0)), side((0.0, 1.0, 0.0)));
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let s = &mut report.pages[0].screens[0];
    assert_eq!((s.pixels_changed, s.regions.len()), (Some(0), 0));
    s.thumb_old = Some("render/0-1-s0-before.webp".into());
    s.thumb_new = Some("render/0-1-s0-after.webp".into());
    let html = render_html(&report);
    let screen = html.split("id=\"page-0-s1\"").nth(1).unwrap();
    assert!(screen.contains("No visible change"), "{screen}");
    assert!(screen.contains("class=\"cmp thumbs\""));
    assert!(screen.contains("src=\"./render/0-1-s0-before.webp\""));
    assert!(screen.contains("src=\"./render/0-1-s0-after.webp\""));
}

#[test]
fn a_small_change_inside_a_wide_layer_is_matched_to_that_layer() {
    // A wide bar is recoloured, but a cover hides all of it except a 6×6 corner: the changed
    // area is far smaller than the bar's box (like one new glyph of a long text line).
    let side = |rgb| {
        let mut bar = filled_rectangle_at(3, 2, rgb, 20.0, 100.0, 260.0, 40.0);
        bar.data.set("name", Val::Str("Bar".into()));
        let bar = Node::from_data(bar.data).unwrap();
        let mut cover = filled_rectangle_at(4, 2, (0.5, 0.5, 0.5), 26.0, 100.0, 254.0, 40.0);
        cover.data.set("name", Val::Str("Cover".into()));
        let mut cover = Node::from_data(cover.data).unwrap();
        cover.data.set("type", Val::Str("RECTANGLE".into()));
        let low = filled_rectangle_at(5, 2, (0.5, 0.5, 0.5), 20.0, 106.0, 6.0, 34.0);
        document(vec![
            screen(),
            bar,
            Node::from_data(cover.data).unwrap(),
            low,
        ])
    };
    let (old, new) = (side((1.0, 0.0, 0.0)), side((0.0, 1.0, 0.0)));
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let s = &report.pages[0].screens[0];
    assert_eq!(s.pixels_changed, Some(36), "{s:?}");
    assert_eq!(s.regions[0].nodes, vec![figstudio_fig::Guid(0, 3)]);
    let html = render_html(&report);
    let area = html.split("id=\"page-0-s1-a1\"").nth(1).unwrap();
    let area = area.split("</article>").next().unwrap();
    assert!(area.contains("class=\"nm\">Bar<"), "{area}");
    assert!(!html.contains("with no matched change line"), "{html}");
}

#[test]
fn a_small_change_at_the_screen_edge_goes_to_the_layer_that_drew_it() {
    // At the top edge the area's padding is one-sided, so its centre falls below a thin bar.
    // A wider covered layer also changed fill but shows no pixel. Each changed pixel goes to
    // the smallest changed layer holding it, and every layer that wins a pixel is kept.
    let side = |rgb| {
        // Stacking follows the position strings: Under, Bar, then the grey covers.
        let named = |mut n: Node, name: &str| {
            n.data.set("name", Val::Str(name.into()));
            let position = match name {
                "Under" => "a",
                "Bar" => "b",
                "Cover" => "c",
                "Dot" => "bb",
                "Low" => "d",
                _ => "e",
            };
            n.data.set(
                "parentIndex",
                support::object(&[
                    ("guid", figstudio_fig::Guid(0, 2).to_val()),
                    ("position", Val::Str(position.into())),
                ]),
            );
            Node::from_data(n.data).unwrap()
        };
        document(vec![
            screen(),
            named(
                filled_rectangle_at(6, 2, rgb, 10.0, 0.0, 280.0, 60.0),
                "Under",
            ),
            named(
                filled_rectangle_at(3, 2, rgb, 20.0, 0.0, 260.0, 10.0),
                "Bar",
            ),
            named(
                filled_rectangle_at(4, 2, (0.5, 0.5, 0.5), 26.0, 0.0, 264.0, 60.0),
                "Cover",
            ),
            named(
                filled_rectangle_at(5, 2, (0.5, 0.5, 0.5), 10.0, 6.0, 16.0, 54.0),
                "Low",
            ),
            named(
                filled_rectangle_at(7, 2, (0.5, 0.5, 0.5), 10.0, 0.0, 10.0, 6.0),
                "Left",
            ),
            // A smaller changed layer over part of the bar's visible corner owns those pixels;
            // the bar keeps the rest, so both are on the area.
            named(filled_rectangle_at(8, 2, rgb, 22.0, 0.0, 4.0, 4.0), "Dot"),
        ])
    };
    let (old, new) = (side((1.0, 0.0, 0.0)), side((0.0, 1.0, 0.0)));
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let s = &report.pages[0].screens[0];
    assert_eq!(s.pixels_changed, Some(36), "{s:?}");
    assert_eq!(s.regions.len(), 1);
    let mut nodes = s.regions[0].nodes.clone();
    nodes.sort();
    assert_eq!(
        nodes,
        vec![figstudio_fig::Guid(0, 3), figstudio_fig::Guid(0, 8)]
    );
}

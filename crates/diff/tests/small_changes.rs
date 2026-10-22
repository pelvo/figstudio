//! A small visible edit (a nudged icon, one changed character) is never dropped as noise
//! when it is the only thing that changed on a screen.
mod support;
use figstudio_diff::{RenderOpts, ScreenStatus, attach_render_diffs, diff_documents, render_html};
use figstudio_fig::{Node, Val};
use support::{document, filled_rectangle_at};

fn screen() -> Node {
    let mut n = filled_rectangle_at(2, 1, (1.0, 1.0, 1.0), 0.0, 0.0, 300.0, 200.0);
    n.data.set("type", Val::Str("FRAME".into()));
    n.data.set("name", Val::Str("Screen".into()));
    Node::from_data(n.data).unwrap()
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

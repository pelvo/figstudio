mod support;
use figstudio_diff::{RenderOpts, attach_render_diffs, diff_documents, render_html};
use figstudio_fig::{Guid, Node, Val};
use support::{filled_rectangle_at, two_page_document};

fn crop_at(x: f64, y: f64) -> figstudio_diff::Region {
    let side = |new: bool| {
        let mut screen = filled_rectangle_at(2, 1, (1.0, 1.0, 1.0), 0.0, 0.0, 800.0, 600.0);
        screen.data.set("type", Val::Str("FRAME".into()));
        let dot = if new {
            (0.0, 0.0, 0.0)
        } else {
            (1.0, 0.0, 0.0)
        };
        two_page_document(
            vec![
                Node::from_data(screen.data).unwrap(),
                filled_rectangle_at(3, 2, dot, x, y, 10.0, 10.0),
            ],
            Vec::new(),
        )
    };
    let (old, new) = (side(false), side(true));
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    report.pages[0].screens[0].regions[0].clone()
}

#[test]
fn a_change_at_the_screen_edge_keeps_the_full_crop_size() {
    for (x, y) in [
        (5.0, 300.0),
        (785.0, 300.0),
        (400.0, 2.0),
        (400.0, 588.0),
        (3.0, 3.0),
    ] {
        let r = crop_at(x, y);
        // 2x pictures: shifted inward, never shrunk to the clamped box.
        assert_eq!(
            (r.image_width, r.image_height),
            (Some(640), Some(320)),
            "{x},{y}"
        );
    }
}

#[test]
fn a_tiny_change_is_shown_with_context_at_natural_size() {
    let side = |new: bool| {
        let mut screen = filled_rectangle_at(2, 1, (1.0, 1.0, 1.0), 0.0, 0.0, 800.0, 600.0);
        screen.data.set("type", Val::Str("FRAME".into()));
        let dot = if new {
            (0.0, 0.0, 0.0)
        } else {
            (1.0, 0.0, 0.0)
        };
        two_page_document(
            vec![
                Node::from_data(screen.data).unwrap(),
                filled_rectangle_at(3, 2, dot, 400.0, 300.0, 10.0, 10.0),
            ],
            Vec::new(),
        )
    };
    let (old, new) = (side(false), side(true));
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let region = report.pages[0].screens[0].regions[0].clone();
    // The box stays the change's own (for the locator); the picture grows to 320x160 at 2x.
    assert!(region.width < 100 && region.height < 100);
    assert_eq!(
        (region.image_width, region.image_height),
        (Some(640), Some(320))
    );
    let crop = image::load_from_memory(&images[&Guid(0, 1)].screens[0].regions[0].after).unwrap();
    assert_eq!((crop.width(), crop.height()), (640, 320));
    for (r, name) in report.pages[0].screens[0]
        .regions
        .iter_mut()
        .zip(["a", "b"])
    {
        r.old_png = Some(format!("render/{name}-before.png"));
        r.new_png = Some(format!("render/{name}-after.png"));
    }
    let html = render_html(&report);
    assert!(html.contains("width=\"320\" height=\"160\""));
    assert!(!html.contains("width=\"640\""));
}

#[test]
fn a_downscaled_screen_keeps_the_same_visible_context() {
    let side = |new: bool| {
        let mut screen = filled_rectangle_at(2, 1, (1.0, 1.0, 1.0), 0.0, 0.0, 6000.0, 3000.0);
        screen.data.set("type", Val::Str("FRAME".into()));
        let dot = if new {
            (0.0, 0.0, 0.0)
        } else {
            (1.0, 0.0, 0.0)
        };
        two_page_document(
            vec![
                Node::from_data(screen.data).unwrap(),
                filled_rectangle_at(3, 2, dot, 3000.0, 1500.0, 20.0, 20.0),
            ],
            Vec::new(),
        )
    };
    let (old, new) = (side(false), side(true));
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let screen = &report.pages[0].screens[0];
    // 18 MP screen is drawn at about 0.48; 320 screen px of context is about 660 design px, so
    // the 2x picture is well over 1000 px wide (the old design-scaled rule gave 640).
    assert!(screen.width < 3000, "screen was not downscaled");
    let w = screen.regions[0].image_width.unwrap();
    assert!(w > 1000, "context shrank with the downscale: {w}");
}

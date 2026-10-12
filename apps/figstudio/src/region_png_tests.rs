//! The PNG size cap, the toast text and a real render of a region of `structure.fig`.

use figstudio_fig::Document;

use super::*;
use figstudio_mcp_core::region_shot::{MAX_PIXELS, MAX_SIDE, Plan};

const STRUCTURE: &[u8] = include_bytes!("../../../tests/fixtures/structure.fig");

fn within_caps(p: Plan) -> bool {
    let (w, h) = (f64::from(p.width), f64::from(p.height));
    w <= MAX_SIDE && h <= MAX_SIDE && w * h <= MAX_PIXELS
}

#[test]
fn a_small_area_keeps_the_asked_scale() {
    let p = plan(100.0, 50.0, 2.0).unwrap();
    assert_eq!((p.width, p.height), (200, 100));
    assert!((p.scale - 2.0).abs() < 1e-9);
}

#[test]
fn the_longest_side_is_capped() {
    // 8000 x 100 at 1x: the pixel total is small, so only the side cap applies.
    let p = plan(8000.0, 100.0, 1.0).unwrap();
    assert!(within_caps(p), "{p:?}");
    assert_eq!(p.width, 4096);
}

#[test]
fn the_pixel_total_is_capped() {
    // 4000 x 4000 fits the side cap (16 MP is exactly 4000 x 4000), 4096 x 4096 does not.
    let p = plan(4096.0, 4096.0, 1.0).unwrap();
    assert!(within_caps(p), "{p:?}");
    assert!(f64::from(p.width) * f64::from(p.height) <= MAX_PIXELS);
    assert!(p.width < 4096);
}

#[test]
fn huge_and_odd_areas_stay_within_the_caps() {
    for (w, h, s) in [
        (1e6, 1e6, 8.0),
        (0.001, 5000.0, 256.0),
        (3999.9, 4000.1, 1.0),
        (1.0, 1.0, 256.0),
    ] {
        let p = plan(w, h, s).unwrap_or_else(|| panic!("{w}x{h}@{s}"));
        assert!(within_caps(p), "{w}x{h}@{s}: {p:?}");
    }
}

#[test]
fn a_bad_area_or_scale_has_no_plan() {
    for (w, h, s) in [
        (0.0, 10.0, 1.0),
        (10.0, -1.0, 1.0),
        (f64::NAN, 10.0, 1.0),
        (10.0, f64::INFINITY, 1.0),
        (10.0, 10.0, 0.0),
        (10.0, 10.0, f64::NAN),
    ] {
        assert_eq!(plan(w, h, s), None, "{w} {h} {s}");
    }
}

#[test]
fn the_toast_names_the_size_and_any_gaps() {
    assert_eq!(message(640, 480, ""), "Copied 640×480 image");
    assert_eq!(
        message(2, 3, "1 effects skipped"),
        "Copied 2×3 image · ⚠ 1 effects skipped"
    );
}

#[test]
fn gaps_keep_only_what_the_status_bar_warns_about() {
    let stats = Stats {
        nodes: 4,
        skipped_effects: 2,
        masks: 1,
        ..Stats::default()
    };
    assert_eq!(gaps(&stats), "2 effects skipped");
    assert_eq!(gaps(&Stats::default()), "");
}

#[test]
fn a_region_renders_to_the_planned_size_and_a_decodable_png() {
    let doc = Document::from_bytes(STRUCTURE).unwrap();
    let page = doc.pages[0];
    let b = figstudio_render::content_bounds(&doc, page).unwrap();
    let job = Job::Region {
        page,
        rect: [b.x0, b.y0, b.width(), b.height()],
        zoom: 1.0,
    };
    let done = render(Images::default(), &doc, None, job, true).unwrap();
    assert_eq!(done.rgba.len(), done.width * done.height * 4);
    assert!(done.rgba.iter().any(|&v| v != 0), "an empty image");
    let png = image::load_from_memory(done.png.as_deref().unwrap()).unwrap();
    assert_eq!(
        (png.width() as usize, png.height() as usize),
        (done.width, done.height)
    );
    assert_eq!(done.width, b.width().ceil() as usize);
    assert_eq!(done.name, "region");
}

#[test]
fn a_bad_job_is_an_error_not_a_panic() {
    let doc = Document::from_bytes(STRUCTURE).unwrap();
    let page = doc.pages[0];
    let empty = Job::Region {
        page,
        rect: [0.0, 0.0, 0.0, 0.0],
        zoom: 1.0,
    };
    assert!(render(Images::default(), &doc, None, empty, false).is_err());
    let missing = Job::Node(figstudio_fig::Guid(99, 99));
    assert!(render(Images::default(), &doc, None, missing, false).is_err());
}

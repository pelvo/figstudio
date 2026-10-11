//! A page recorded once per zoom and replayed per tile draws what a direct render of each
//! tile draws (feature `parallel`).
#![cfg(feature = "parallel")]
mod common;
use common::*;
use figstudio_fig::{Document, Guid, Val};
use figstudio_render::kurbo::{Affine, Rect};
use figstudio_render::{Recording, Renderer, Viewport};

const TILE: f64 = 512.0;
const GUTTER: f64 = 2.0;
const SIDE: u16 = 516;

fn open(name: &str) -> Document {
    Document::open(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(format!("{name}.fig")),
    )
    .unwrap()
}

fn renderer() -> Renderer {
    let mut r = Renderer::new();
    r.system_fonts = false;
    r
}

/// The app's tile viewport (`tiles::Key::transform`): tile `(x, y)` at `scale`, with gutter.
fn tile(scale: f64, x: i32, y: i32) -> Viewport {
    Viewport {
        width: SIDE,
        height: SIDE,
        transform: Affine::translate((GUTTER - f64::from(x) * TILE, GUTTER - f64::from(y) * TILE))
            * Affine::scale(scale),
        background: Some(figstudio_render::color::AlphaColor::new([
            1.0, 1.0, 1.0, 1.0,
        ])),
    }
}

/// Largest channel difference between the replayed and the direct render of a tile, and
/// whether the direct render drew anything but the background.
fn compare(doc: &Document, page: Guid, rec: &Recording, scale: f64, x: i32, y: i32) -> (u8, bool) {
    let vp = tile(scale, x, y);
    let direct = renderer().render(doc, page, &vp);
    let (replayed, _) = rec.render(&vp).expect("same zoom");
    let a = direct.pixmap.data_as_u8_slice();
    let worst = a
        .iter()
        .zip(replayed.pixmap.data_as_u8_slice())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    (worst, a.iter().any(|&c| c != 255))
}

/// Recorded pixels of tiles `-n..n` each way.
fn tiles_around(n: i32) -> Rect {
    let e = f64::from(n) * TILE + 2.0 * GUTTER;
    Rect::new(-e, -e, e, e)
}

fn record(doc: &Document, page: Guid, scale: f64) -> Recording {
    let area = tiles_around(64);
    let (rec, _) = renderer().record(doc, page, Affine::scale(scale), area, usize::MAX);
    rec.expect("no limit")
}

#[test]
fn fixture_tiles_replay_like_direct_renders() {
    for name in [
        "corners",
        "sections",
        "stacks_wrap",
        "structure",
        "vector",
        "noise_and_grain",
    ] {
        let doc = open(name);
        let page = doc.pages[0];
        let b = figstudio_render::content_bounds(&doc, page).unwrap();
        for scale in [0.5, 2f64.sqrt(), 2.0] {
            let rec = record(&doc, page, scale);
            let k = scale / TILE;
            let (x0, y0) = ((b.x0 * k).floor() as i32, (b.y0 * k).floor() as i32);
            let (x1, y1) = ((b.x1 * k).ceil() as i32, (b.y1 * k).ceil() as i32);
            let mut drawn = 0;
            for y in y0 - 1..=y1.min(y0 + 3) {
                for x in x0 - 1..=x1.min(x0 + 3) {
                    let (worst, ink) = compare(&doc, page, &rec, scale, x, y);
                    assert!(
                        worst <= 1,
                        "{name} at {scale} tile ({x},{y}) differs by {worst}"
                    );
                    drawn += usize::from(ink);
                }
            }
            assert!(drawn > 0, "{name} at {scale}: no tile had content");
        }
    }
}

#[test]
fn effects_clips_and_negative_tiles_replay_like_direct_renders() {
    let doc = effects_doc();
    for scale in [0.75, 1.0, 2f64.sqrt(), 3.0] {
        let rec = record(&doc, id(1), scale);
        assert!(rec.stats().shadows + rec.stats().blurs > 0, "effects drawn");
        for y in -2..=1 {
            for x in -2..=1 {
                let (worst, _) = compare(&doc, id(1), &rec, scale, x, y);
                assert!(worst <= 1, "{scale} tile ({x},{y}) differs by {worst}");
            }
        }
    }
}

#[test]
fn a_tile_far_from_content_replays_no_fill() {
    let doc = effects_doc();
    let rec = record(&doc, id(1), 1.0);
    let (_, near) = rec.render(&tile(1.0, -1, -1)).unwrap();
    assert!(near > 0, "the tile at the content replays its fills");
    // Beside the content in one axis only; the background is filled by `render` itself.
    for (x, y) in [(40, 0), (0, 40), (-40, -1), (-1, -40)] {
        let (_, far) = rec.render(&tile(1.0, x, y)).unwrap();
        assert_eq!(
            far, 0,
            "tile ({x},{y}) away from all content replays nothing"
        );
    }
}

#[test]
fn another_zoom_or_a_tile_outside_the_area_is_refused() {
    let doc = effects_doc();
    let rec = record(&doc, id(1), 1.0);
    assert!(rec.render(&tile(2.0, 0, 0)).is_none());
    let (small, _) = renderer().record(&doc, id(1), Affine::IDENTITY, tiles_around(1), usize::MAX);
    let small = small.unwrap();
    assert!(small.render(&tile(1.0, -1, -1)).is_some());
    assert!(small.render(&tile(1.0, 0, 0)).is_some());
    for (x, y) in [(1, 0), (0, 1), (-2, 0), (0, -2)] {
        assert!(small.render(&tile(1.0, x, y)).is_none(), "tile ({x},{y})");
    }
}

/// Culling to the recorded area keeps what can reach it: a tile at the area's edge replays
/// like a direct render even when a shadow or blur reaches it from outside.
#[test]
fn a_small_area_replays_like_direct_renders() {
    let doc = effects_doc();
    let (rec, _) = renderer().record(&doc, id(1), Affine::IDENTITY, tiles_around(1), usize::MAX);
    let rec = rec.unwrap();
    for (x, y) in [(-1, -1), (0, -1), (-1, 0), (0, 0)] {
        let (worst, _) = compare(&doc, id(1), &rec, 1.0, x, y);
        assert!(worst <= 1, "tile ({x},{y}) differs by {worst}");
    }
}

#[test]
fn a_recording_over_its_limit_is_dropped() {
    let doc = open("structure");
    let page = doc.pages[0];
    let full = record(&doc, page, 1.0);
    assert!(full.bytes() > 0);
    let area = tiles_around(64);
    let (rec, _) = renderer().record(&doc, page, Affine::scale(1.0), area, full.bytes() - 1);
    assert!(rec.is_none(), "one byte short of the recording");
    let (rec, _) = renderer().record(&doc, page, Affine::scale(1.0), area, full.bytes());
    assert!(rec.is_some(), "exactly the recording fits");
}

/// A blurred parent spreads a child lying just outside a tile into it: a direct render of the
/// tile must not cull that child, so it matches replays of recordings over any area.
#[test]
fn a_child_blurred_in_from_outside_the_tile_is_drawn() {
    let mut parent = node(2, 1, "FRAME");
    parent.data.set("size", size(700.0, 100.0));
    parent.data.set(
        "effects",
        Val::Arr(vec![obj(&[
            ("type", Val::Str("LAYER_BLUR".into())),
            ("visible", Val::Bool(true)),
            ("radius", Val::Float(24.0)),
        ])]),
    );
    // Tile (0, 0) at scale 1 ends at page x 514; the child starts 6 units later.
    let mut child = node(3, 2, "RECTANGLE");
    child.data.set("size", size(60.0, 60.0));
    child.data.set(
        "transform",
        obj(&[("m02", Val::Float(520.0)), ("m12", Val::Float(20.0))]),
    );
    child.data.set("fillPaints", paint(0.0, 0.0, 0.0));
    let doc = document(vec![node(1, 0, "CANVAS"), parent, child]);
    for n in [1, 64] {
        let (rec, _) =
            renderer().record(&doc, id(1), Affine::IDENTITY, tiles_around(n), usize::MAX);
        let (worst, _) = compare(&doc, id(1), &rec.unwrap(), 1.0, 0, 0);
        assert!(worst <= 1, "recorded over {n} tiles: differs by {worst}");
    }
}

/// Paint allocations count toward the limit: a gradient with 20,000 stops (about 400 KB of
/// stops) does not fit a 16 KiB recording.
#[test]
fn many_gradient_stops_exceed_the_limit() {
    let stop = |i: u32| {
        obj(&[
            ("position", Val::Float(f64::from(i) / 20_000.0)),
            (
                "color",
                obj(&[("r", Val::Float(1.0)), ("a", Val::Float(1.0))]),
            ),
        ])
    };
    let mut rect = node(2, 1, "RECTANGLE");
    rect.data.set(
        "fillPaints",
        Val::Arr(vec![obj(&[
            ("type", Val::Str("GRADIENT_LINEAR".into())),
            ("visible", Val::Bool(true)),
            ("stops", Val::Arr((0..20_000).map(stop).collect())),
        ])]),
    );
    let doc = document(vec![node(1, 0, "CANVAS"), rect]);
    let (rec, _) = renderer().record(&doc, id(1), Affine::IDENTITY, tiles_around(1), usize::MAX);
    let bytes = rec.unwrap().bytes();
    assert!(bytes > 20_000 * 16, "the stops are counted: {bytes} bytes");
    let (rec, _) = renderer().record(&doc, id(1), Affine::IDENTITY, tiles_around(1), 16 << 10);
    assert!(rec.is_none(), "over a 16 KiB limit");
}

/// A blur's reach applies to its own node's content only: a later sibling outside the view is
/// still culled.
#[test]
fn a_blur_does_not_keep_its_siblings_from_being_culled() {
    let mut blurred = node(2, 1, "FRAME");
    blurred.data.set(
        "effects",
        Val::Arr(vec![obj(&[
            ("type", Val::Str("LAYER_BLUR".into())),
            ("visible", Val::Bool(true)),
            ("radius", Val::Float(24.0)),
        ])]),
    );
    let mut child = node(3, 2, "RECTANGLE");
    child.data.set("fillPaints", paint(0.0, 0.0, 0.0));
    // 10 units right of an 80-unit-wide view: within the blur's reach, outside the view.
    let mut sibling = node(4, 1, "RECTANGLE");
    sibling
        .data
        .set("transform", obj(&[("m02", Val::Float(90.0))]));
    sibling.data.set("fillPaints", paint(0.0, 0.0, 0.0));
    let doc = document(vec![node(1, 0, "CANVAS"), blurred, child, sibling]);
    let vp = Viewport {
        width: 80,
        height: 80,
        transform: Affine::IDENTITY,
        background: None,
    };
    assert_eq!(renderer().render(&doc, id(1), &vp).stats.culled, 1);
}

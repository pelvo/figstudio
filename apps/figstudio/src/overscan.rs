//! Overscan: the canvas renders a margin around the visible area, so a short pan shows
//! content that is already drawn instead of an empty edge while the next frame renders.

use egui::Vec2;
use figstudio_render::kurbo::Affine;

use crate::tab::View;

/// A requested frame: the view of its own top-left corner, its size and its margin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// The canvas view moved by the margin, so the frame's pixel (0, 0) is the margin's corner.
    pub view: View,
    /// Frame size in pixels (the render viewport).
    pub px: [u16; 2],
    /// Pixels per point on each axis: the screen's, or less when the canvas is over a cap.
    pub scale: Vec2,
    /// Frame size in points, for re-projecting it onto the canvas.
    pub size_points: Vec2,
    /// Margin on each side, in points.
    pub margin: Vec2,
}

impl Frame {
    /// Page-to-frame-pixel transform for the render viewport.
    pub fn transform(&self) -> Affine {
        let (pan, s) = (self.view.pan, self.scale);
        Affine::translate((f64::from(pan.x * s.x), f64::from(pan.y * s.y)))
            * Affine::scale_non_uniform(
                f64::from(self.view.zoom * s.x),
                f64::from(self.view.zoom * s.y),
            )
    }
}

/// Most pixels a frame may have (128 MB of RGBA native, 64 MB in the browser).
#[cfg(not(target_arch = "wasm32"))]
pub const MAX_PIXELS: f32 = 32.0 * 1024.0 * 1024.0;
#[cfg(target_arch = "wasm32")]
pub const MAX_PIXELS: f32 = 16.0 * 1024.0 * 1024.0;

/// The frame to render for `view` on a canvas of `canvas_px` pixels: half the canvas on each
/// side (so at most 4x the canvas area), less when the frame would pass `max_side` (the GPU
/// texture limit), `u16::MAX` (the render viewport) or [`MAX_PIXELS`]. A canvas that is itself
/// over a cap gets no margin and is rendered at a lower resolution, stretched when shown.
pub fn frame_for(view: View, canvas_px: Vec2, ppp: f32, max_side: usize) -> Frame {
    let limit = f32::from(u16::try_from(max_side).unwrap_or(u16::MAX)).max(1.0);
    let ppp = ppp.max(f32::MIN_POSITIVE);
    let canvas_px = canvas_px.round().max(Vec2::splat(1.0));
    let area = canvas_px.x * canvas_px.y;
    let (size_px, margin_px) = if canvas_px.max_elem() <= limit && area <= MAX_PIXELS {
        // (1 + k)^2 * canvas area <= MAX_PIXELS, for margin = k * canvas / 2.
        let k = ((MAX_PIXELS / area).sqrt() - 1.0).clamp(0.0, 1.0);
        let room = ((Vec2::splat(limit) - canvas_px) / 2.0).floor();
        let margin_px = (canvas_px * k / 2.0).floor().min(room).max(Vec2::ZERO);
        (canvas_px + margin_px * 2.0, margin_px)
    } else {
        let shrink = (limit / canvas_px.max_elem()).min((MAX_PIXELS / area).sqrt());
        (
            (canvas_px * shrink).floor().max(Vec2::splat(1.0)),
            Vec2::ZERO,
        )
    };
    let size_points = (canvas_px + margin_px * 2.0) / ppp;
    let margin = margin_px / ppp;
    Frame {
        view: View {
            zoom: view.zoom,
            pan: view.pan + margin,
        },
        // In 1..=u16::MAX by the clamps above.
        px: [size_px.x as u16, size_px.y as u16],
        scale: size_px / size_points,
        size_points,
        margin,
    }
}

/// The frame to request now, or `None` while `last` (the last requested frame for the same
/// page, zoom, canvas size and document) still covers `view`.
pub fn plan(
    last: Option<&Frame>,
    view: View,
    canvas_px: Vec2,
    ppp: f32,
    max_side: usize,
) -> Option<Frame> {
    let canvas_points = canvas_px.round() / ppp.max(f32::MIN_POSITIVE);
    match last {
        Some(frame) if covers(frame, view, canvas_points) => None,
        _ => Some(frame_for(view, canvas_px, ppp, max_side)),
    }
}

/// Whether `frame` still covers a canvas of `canvas_points` shown with `view`, keeping at
/// least a quarter of its margin beyond every edge. A zoom change never counts as covered.
pub fn covers(frame: &Frame, view: View, canvas_points: Vec2) -> bool {
    if frame.view.zoom != view.zoom {
        return false;
    }
    let keep = frame.margin / 4.0 - Vec2::splat(1e-3);
    // The frame's top-left corner in canvas points.
    let min = view.pan - frame.view.pan;
    let max = min + frame.size_points;
    min.x <= -keep.x
        && min.y <= -keep.y
        && max.x >= canvas_points.x + keep.x
        && max.y >= canvas_points.y + keep.y
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(x: f32, y: f32) -> View {
        View {
            zoom: 1.5,
            pan: Vec2::new(x, y),
        }
    }

    #[test]
    fn the_frame_has_half_the_canvas_on_every_side() {
        let f = frame_for(view(10.0, 20.0), Vec2::new(800.0, 600.0), 2.0, 16384);
        assert_eq!(f.px, [1600, 1200]);
        assert_eq!(f.margin, Vec2::new(200.0, 150.0));
        assert_eq!(f.size_points, Vec2::new(800.0, 600.0));
        assert_eq!(f.view.pan, Vec2::new(210.0, 170.0));
        assert_eq!(f.view.zoom, 1.5);
        let area = f32::from(f.px[0]) * f32::from(f.px[1]);
        assert!(area <= 4.0 * 800.0 * 600.0);
    }

    #[test]
    fn the_margin_shrinks_to_the_texture_limit_and_u16() {
        let f = frame_for(view(0.0, 0.0), Vec2::new(3000.0, 1000.0), 1.0, 4096);
        assert_eq!(f.px, [4096, 2000]);
        assert_eq!(f.margin, Vec2::new(548.0, 500.0));
        let f = frame_for(view(0.0, 0.0), Vec2::new(100.0, 60000.0), 1.0, 100_000);
        assert_eq!(f.px, [200, 65534]);
        // A canvas at the limit gets no margin and is still rendered.
        let f = frame_for(view(0.0, 0.0), Vec2::new(4096.0, 100.0), 1.0, 4096);
        assert_eq!(f.px, [4096, 200]);
        assert_eq!(f.margin.x, 0.0);
    }

    #[test]
    fn a_big_canvas_keeps_the_frame_within_the_pixel_cap() {
        // A 4K canvas at 2x: half-canvas margins would make 15360 x 8640 = 133 Mpx.
        let f = frame_for(view(0.0, 0.0), Vec2::new(7680.0, 4320.0), 2.0, 16384);
        let area = f32::from(f.px[0]) * f32::from(f.px[1]);
        assert!(area <= MAX_PIXELS, "{:?}", f.px);
        assert!(f.margin.x > 0.0 && f.margin.y > 0.0, "{:?}", f.margin);
    }

    #[test]
    fn plan_requests_only_when_the_last_frame_stops_covering() {
        let canvas = Vec2::new(400.0, 300.0);
        let first = plan(None, view(0.0, 0.0), canvas, 1.0, 8192).expect("first frame");
        assert_eq!(first, frame_for(view(0.0, 0.0), canvas, 1.0, 8192));
        for _ in 0..3 {
            assert_eq!(plan(Some(&first), view(0.0, 0.0), canvas, 1.0, 8192), None);
        }
        assert_eq!(
            plan(Some(&first), view(100.0, 0.0), canvas, 1.0, 8192),
            None
        );
        let next = plan(Some(&first), view(160.0, 0.0), canvas, 1.0, 8192).expect("near edge");
        assert_eq!(next.view.pan, Vec2::new(360.0, 150.0));
    }

    #[test]
    fn a_pan_inside_the_margin_needs_no_new_frame() {
        let canvas = Vec2::new(400.0, 300.0);
        let f = frame_for(view(0.0, 0.0), canvas, 1.0, 8192);
        assert!(covers(&f, view(0.0, 0.0), canvas));
        assert!(covers(&f, view(100.0, -80.0), canvas));
        assert!(covers(&f, view(-150.0, 112.0), canvas));
    }

    #[test]
    fn a_pan_near_the_frame_edge_needs_a_new_frame() {
        let canvas = Vec2::new(400.0, 300.0);
        let f = frame_for(view(0.0, 0.0), canvas, 1.0, 8192);
        // Margin 200 x 150; a quarter of it (50 x 37.5) must stay beyond the edge.
        assert!(!covers(&f, view(151.0, 0.0), canvas));
        assert!(!covers(&f, view(-151.0, 0.0), canvas));
        assert!(!covers(&f, view(0.0, 113.0), canvas));
        assert!(!covers(&f, view(0.0, -113.0), canvas));
    }

    #[test]
    fn a_zoom_change_or_no_margin_needs_a_new_frame() {
        let canvas = Vec2::new(400.0, 300.0);
        let f = frame_for(view(0.0, 0.0), canvas, 1.0, 8192);
        let zoomed = View {
            zoom: 3.0,
            pan: Vec2::ZERO,
        };
        assert!(!covers(&f, zoomed, canvas));
        // Without room for a margin, only the exact view is covered.
        let f = frame_for(view(0.0, 0.0), canvas, 1.0, 400);
        assert!(covers(&f, view(0.0, 0.0), canvas));
        assert!(!covers(&f, view(1.0, 0.0), canvas));
    }

    #[test]
    fn the_transform_maps_the_covered_page_area_onto_the_frame_pixels() {
        for (canvas, ppp, max_side) in [
            (Vec2::new(400.0, 300.0), 2.0, 8192),
            (Vec2::new(8192.0, 8192.0), 2.0, 16384),
            // Shrunk with per-axis rounding: the x and y scales differ.
            (Vec2::new(4100.0, 1000.0), 1.0, 4096),
            (Vec2::new(1001.0, 5003.0), 2.625, 4096),
        ] {
            let f = frame_for(view(30.0, -12.0), canvas, ppp, max_side);
            let t = f.transform();
            // The frame's corners in page space: screen = pan + page * zoom (points).
            let page = |p: Vec2| (p - f.view.pan) / f.view.zoom;
            let (a, b) = (page(Vec2::ZERO), page(f.size_points));
            let p0 = t * figstudio_render::kurbo::Point::new(f64::from(a.x), f64::from(a.y));
            let p1 = t * figstudio_render::kurbo::Point::new(f64::from(b.x), f64::from(b.y));
            assert!(p0.x.abs() < 0.01 && p0.y.abs() < 0.01, "{canvas:?}: {p0:?}");
            assert!(
                (p1.x - f64::from(f.px[0])).abs() < 0.01,
                "{canvas:?}: {p1:?} {:?}",
                f.px
            );
            assert!(
                (p1.y - f64::from(f.px[1])).abs() < 0.01,
                "{canvas:?}: {p1:?} {:?}",
                f.px
            );
        }
    }

    #[test]
    fn a_canvas_over_the_caps_is_rendered_smaller_and_covers_itself() {
        // (canvas px, ppp, max side): over the pixel cap, over the texture side, odd scales.
        let cases = [
            (Vec2::new(8192.0, 8192.0), 2.0, 16384),
            (Vec2::new(4100.0, 1000.0), 1.0, 4096),
            (Vec2::new(1000.0, 5000.0), 2.625, 4096),
            (Vec2::new(70000.0, 300.0), 1.0, 100_000),
            (Vec2::new(1234.0, 777.0), 1.25, 8192),
        ];
        for (canvas, ppp, max_side) in cases {
            let v = view(3.25, -7.5);
            let f = frame_for(v, canvas, ppp, max_side);
            let area = f32::from(f.px[0]) * f32::from(f.px[1]);
            assert!(area <= MAX_PIXELS, "{canvas:?}: {:?}", f.px);
            assert!(
                usize::from(f.px[0].max(f.px[1])) <= max_side,
                "{canvas:?}: {:?}",
                f.px
            );
            // Right after planning, the same view must not ask for another frame.
            assert!(covers(&f, v, canvas / ppp), "{canvas:?} at {ppp}: {f:?}");
            assert_eq!(plan(Some(&f), v, canvas, ppp, max_side), None, "{canvas:?}");
            let need = canvas.round() / ppp;
            assert!(
                f.size_points.x >= need.x && f.size_points.y >= need.y,
                "{canvas:?}"
            );
        }
    }
}

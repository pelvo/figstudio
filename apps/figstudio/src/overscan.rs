//! Overscan: the canvas renders a margin around the visible area, so a short pan shows
//! content that is already drawn instead of an empty edge while the next frame renders.

use egui::Vec2;

use crate::tab::View;

/// A requested frame: the view of its own top-left corner, its size and its margin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// The canvas view moved by the margin, so the frame's pixel (0, 0) is the margin's corner.
    pub view: View,
    /// Frame size in pixels (the render viewport).
    pub px: [u16; 2],
    /// Frame size in points, for re-projecting it onto the canvas.
    pub size_points: Vec2,
    /// Margin on each side, in points.
    pub margin: Vec2,
}

/// Most pixels a frame may have (128 MB of RGBA), so a big high-DPI canvas keeps a smaller margin.
pub const MAX_PIXELS: f32 = 32.0 * 1024.0 * 1024.0;

/// The frame to render for `view` on a canvas of `canvas_px` pixels: half the canvas on each
/// side (so at most 4x the canvas area), less when the frame would pass `max_side` (the GPU
/// texture limit), `u16::MAX` (the render viewport) or [`MAX_PIXELS`].
pub fn frame_for(view: View, canvas_px: Vec2, ppp: f32, max_side: usize) -> Frame {
    let limit = f32::from(u16::try_from(max_side).unwrap_or(u16::MAX));
    let canvas_px = canvas_px.min(Vec2::splat(limit)).max(Vec2::ZERO).round();
    let room = ((Vec2::splat(limit) - canvas_px) / 2.0).floor();
    // (1 + k)^2 * canvas area <= MAX_PIXELS, for margin = k * canvas / 2.
    let k = ((MAX_PIXELS / (canvas_px.x * canvas_px.y).max(1.0)).sqrt() - 1.0).clamp(0.0, 1.0);
    let margin_px = (canvas_px * k / 2.0).floor().min(room).max(Vec2::ZERO);
    let size_px = canvas_px + margin_px * 2.0;
    let ppp = ppp.max(f32::MIN_POSITIVE);
    Frame {
        view: View {
            zoom: view.zoom,
            pan: view.pan + margin_px / ppp,
        },
        // In 0..=u16::MAX by the clamps above.
        px: [size_px.x as u16, size_px.y as u16],
        size_points: size_px / ppp,
        margin: margin_px / ppp,
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
}

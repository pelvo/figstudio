//! A conservative estimate of the memory a `vello_gpu` 0.3 `Scene` keeps for one tile, made by
//! the support scan before the scene is built (`vello_gpu::Scene` has no size accessor: its
//! strip storage, recorder and encoded paints are `pub(crate)`, `vello_gpu` `scene.rs`).
//!
//! What it counts, from `vello_common` 0.3 (`strip.rs`, `strip_generator.rs`, `tile.rs`,
//! `flatten.rs`, `encode.rs`) and `vello_gpu` 0.3 (`scene.rs`):
//! - Strips and alphas, kept for every path (fills and clips) in the scene's strip storage: an
//!   8-byte `Strip` and 16 alpha bytes (4 × 4 px) per tile a path's outline touches inside the
//!   viewport, at most the tiles of its bounding box there; a sentinel strip per path; and for
//!   geometry left of the viewport, two strips and 16 alphas per row (`emit_culled_background`).
//! - The strip generator's buffers, kept at the capacity of the largest path: 16 bytes per
//!   flattened `Line` and 8 per `Tile` entry (one per line and tile it touches).
//! - The recorder's draw and layer entries, and each fill's encoded gradient (its ranges).
//!
//! Outlines are flattened with `kurbo` at a coarser tolerance than `vello_common`'s (0.25 px),
//! which yields fewer, slightly shorter lines; the margin below covers both. Every buffer is a
//! `Vec` that doubles when full, so the total is doubled. Measured against the live heap of
//! built scenes (one 516 px tile of synthetic pages: 10 to 3,000 ellipses, rectangles, stroked
//! ellipses and 16-stop gradients, 40 to 2,000 px wide): the estimate was 1.9 to 5 times the
//! real size (1.3 MiB to 20 MiB), never below it.

use vello_cpu::PaintType;
use vello_cpu::kurbo::{self, Affine, BezPath, Rect, Shape};

/// `vello_common`'s tile side in pixels.
const TILE: f64 = 4.0;
/// Tolerance of the estimate's flattening (pixels).
const TOLERANCE: f64 = 1.0;
/// `vello_common` flattens at 0.25 px: about twice the lines of a 1 px flattening.
const LINES_AT_VELLO_TOLERANCE: f64 = 2.0;
/// Margin over the measured outline length (lines shorter than the curve, rounding).
const MARGIN: f64 = 1.25;
/// Bytes per outline tile kept: an 8-byte strip and 16 alpha bytes.
const PER_TILE: f64 = 24.0;
/// Bytes per recorded draw or layer (the draw, its paint and its node in the recorder).
const PER_DRAW: f64 = 128.0;
/// Growth slack: the scene's `Vec`s double when full, so up to twice their length is kept.
const CAPACITY: f64 = 2.0;

/// The running estimate (see the module docs).
#[derive(Debug, Default)]
pub(crate) struct SceneBytes {
    kept: f64,
    /// The strip generator's buffers for the largest path.
    largest_path: f64,
    /// Bytes each fill with the current paint adds (its encoded gradient).
    paint: f64,
}

impl SceneBytes {
    /// The estimate, in bytes.
    pub(crate) fn total(&self) -> usize {
        let total = CAPACITY * (self.kept + self.largest_path);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a non-negative byte count; saturates"
        )]
        let bytes = total.ceil() as usize;
        bytes
    }

    pub(crate) fn set_paint(&mut self, paint: &PaintType) {
        self.paint = match paint {
            PaintType::Gradient(g) => 256.0 + 64.0 * g.stops.len() as f64,
            _ => 0.0,
        };
    }

    /// A fill of `path` under `transform` in a viewport of `view` (pixels).
    pub(crate) fn fill(&mut self, path: &BezPath, transform: Affine, view: Rect) {
        self.kept += self.paint;
        self.path(path, transform, view);
    }

    /// A pushed layer, with its clip `path` if any.
    pub(crate) fn layer(&mut self, path: Option<&BezPath>, transform: Affine, view: Rect) {
        match path {
            Some(path) => self.path(path, transform, view),
            None => self.kept += PER_DRAW,
        }
    }

    fn path(&mut self, path: &BezPath, transform: Affine, view: Rect) {
        self.kept += PER_DRAW + 8.0;
        let bounds = (transform * path).bounding_box();
        // Tiles the outline can touch: the view, one tile larger on each side.
        let near = view.inflate(TILE, TILE);
        let (mut touched, mut lines, mut pairs) = (0.0, 0.0, 0.0);
        let (mut start, mut last) = (kurbo::Point::ZERO, kurbo::Point::ZERO);
        let mut line = |a: kurbo::Point, b: kurbo::Point| {
            lines += 1.0;
            let tiles = ((b.x - a.x).abs() + (b.y - a.y).abs()) / TILE + 2.0;
            pairs += tiles;
            let seg = Rect::from_points(a, b);
            if seg.x1 >= near.x0 && seg.x0 <= near.x1 && seg.y1 >= near.y0 && seg.y0 <= near.y1 {
                // The part inside spans at most the clamped ends' extent on each axis.
                let c = |p: kurbo::Point| {
                    kurbo::Point::new(p.x.clamp(near.x0, near.x1), p.y.clamp(near.y0, near.y1))
                };
                let (a, b) = (c(a), c(b));
                touched += ((b.x - a.x).abs() + (b.y - a.y).abs()) / TILE + 2.0;
            }
        };
        kurbo::flatten(transform * path, TOLERANCE, |el| match el {
            kurbo::PathEl::MoveTo(p) => {
                line(last, start);
                (start, last) = (p, p);
            }
            kurbo::PathEl::LineTo(p) => {
                line(last, p);
                last = p;
            }
            kurbo::PathEl::ClosePath => {
                line(last, start);
                last = start;
            }
            // `flatten` emits only lines.
            kurbo::PathEl::QuadTo(..) | kurbo::PathEl::CurveTo(..) => {}
        });
        line(last, start);
        let inside = bounds.intersect(near);
        let rows = if inside.height() > 0.0 {
            inside.height() / TILE + 2.0
        } else {
            0.0
        };
        let area = if inside.area() > 0.0 {
            (inside.width() / TILE + 2.0) * rows
        } else {
            0.0
        };
        let culled = if bounds.x0 < view.x0 {
            rows * 32.0
        } else {
            0.0
        };
        self.kept += (MARGIN * touched).min(area) * PER_TILE + culled;
        let lines = LINES_AT_VELLO_TOLERANCE * MARGIN * lines;
        let generator = 16.0 * lines + 8.0 * MARGIN * (pairs + lines);
        self.largest_path = self.largest_path.max(generator);
    }
}

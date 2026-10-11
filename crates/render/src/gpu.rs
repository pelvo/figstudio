//! Replaying a recorded tile into a `vello_gpu::Scene` (feature `gpu`). The same command list
//! as the CPU replay ([`Recording::render`]) is replayed, first into a scan that refuses what
//! `vello_gpu` 0.3 cannot draw (it panics on those), then into the scene.
//!
//! `vello_gpu` 0.3 API used: `Scene::{new, set_transform, set_paint, set_paint_transform,
//! set_fill_rule, fill_path, fill_rect, push_clip_layer, push_clip_path, pop_clip,
//! push_opacity_layer, push_filter_layer, pop_layer}`. Rendering the scene (`Renderer::render`)
//! is the caller's: the app into its canvas texture, `tests/gpu.rs` into a texture read back.

use std::fmt;

use vello_cpu::PaintType;
use vello_cpu::filter_effects::{Filter, FilterPrimitive};
use vello_cpu::kurbo::{Affine, BezPath, Rect};
use vello_cpu::peniko::{Fill, GradientKind};
use vello_gpu::Scene;

use crate::Viewport;
use crate::record::Canvas;
use crate::replay::Recording;

/// Why a tile cannot be drawn by `vello_gpu`; draw it on the CPU instead and count it in
/// [`crate::Stats::gpu_fallback_tiles`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GpuUnsupported {
    /// An image paint: `vello_gpu` needs images uploaded to its atlas first (it panics on the
    /// pixmap images the CPU renderer draws).
    Image,
    /// An angular (sweep) gradient: `vello_gpu` 0.3 draws the start colour along its seam where
    /// the CPU draws the end colour (a dotted line of wrong pixels, measured in `tests/gpu.rs`).
    SweepGradient,
    /// A filter other than one blur, drop shadow, offset or flood (`vello_gpu` panics on
    /// multi-primitive graphs and other primitives).
    FilterGraph,
    /// A filter spreading further than [`MAX_FILTER_REACH`] pixels: its layer would exceed the
    /// GPU's intermediate textures.
    FilterReach,
    /// More than [`MAX_LAYERS`] layers in the tile (`vello_gpu`'s scheduler gives up, with a
    /// panic, on extreme scenes; its filter data texture has a fixed size).
    Layers,
}

impl fmt::Display for GpuUnsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            GpuUnsupported::Image => "image paint",
            GpuUnsupported::SweepGradient => "angular gradient",
            GpuUnsupported::FilterGraph => "filter not supported on the GPU",
            GpuUnsupported::FilterReach => "filter reaches too far for the GPU",
            GpuUnsupported::Layers => "too many layers for the GPU",
        })
    }
}

impl std::error::Error for GpuUnsupported {}

/// Largest filter expansion, in output pixels each way, drawn on the GPU (its intermediate
/// textures are at most 4096 px; a 516 px tile plus twice this stays inside).
pub const MAX_FILTER_REACH: f64 = 1024.0;
/// Most layers (clip, opacity and filter) one GPU tile may push.
pub const MAX_LAYERS: usize = 10_000;

impl Recording {
    /// Builds the `vello_gpu` scene of `viewport`, as [`Recording::render`] would draw it on the
    /// CPU (with its background). Returns the scene and how many fills were replayed,
    /// `Some(Err(_))` when the tile uses something `vello_gpu` cannot draw (draw it with
    /// [`Recording::render`]), or `None` when [`Recording::render`] would.
    pub fn gpu_scene(&self, viewport: &Viewport) -> Option<Result<(Scene, usize), GpuUnsupported>> {
        let (window, shift) = self.place(viewport)?;
        let mut scan = Scan::default();
        self.rec.replay(&mut scan, window, shift);
        if let Some(why) = scan.unsupported {
            return Some(Err(why));
        }
        let (w, h) = (viewport.width.max(1), viewport.height.max(1));
        let mut scene = Scene::new(w, h);
        if let Some(bg) = viewport.background {
            scene.set_paint(bg);
            scene.fill_rect(&Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
        }
        let fills = self.rec.replay(&mut scene, window, shift);
        Some(Ok((scene, fills)))
    }
}

/// Checks the commands a tile replays; keeps the first reason it cannot go to the GPU.
#[derive(Default)]
struct Scan {
    transform: Affine,
    layers: usize,
    unsupported: Option<GpuUnsupported>,
}

impl Scan {
    fn refuse(&mut self, why: GpuUnsupported) {
        self.unsupported.get_or_insert(why);
    }

    fn layer(&mut self) {
        self.layers += 1;
        if self.layers > MAX_LAYERS {
            self.refuse(GpuUnsupported::Layers);
        }
    }
}

/// Whether `vello_gpu` 0.3 draws `filter` (`vello_common::filter::PreparedFilter::new`).
fn supported_filter(filter: &Filter) -> bool {
    matches!(
        filter.graph.primitives.as_slice(),
        [FilterPrimitive::GaussianBlur { .. }
            | FilterPrimitive::DropShadow { .. }
            | FilterPrimitive::DropShadowOnly { .. }
            | FilterPrimitive::Offset { .. }
            | FilterPrimitive::Flood { .. }]
    )
}

fn reach(r: Rect) -> f64 {
    [r.x0, r.y0, r.x1, r.y1]
        .into_iter()
        .map(f64::abs)
        .fold(0.0, f64::max)
}

impl Canvas for Scan {
    fn set_transform(&mut self, t: Affine) {
        self.transform = t;
    }
    fn set_paint(&mut self, paint: PaintType) {
        match paint {
            PaintType::Image(_) => self.refuse(GpuUnsupported::Image),
            PaintType::Gradient(g) if matches!(g.kind, GradientKind::Sweep(_)) => {
                self.refuse(GpuUnsupported::SweepGradient);
            }
            _ => {}
        }
    }
    fn set_paint_transform(&mut self, _: Affine) {}
    fn set_fill_rule(&mut self, _: Fill) {}
    fn fill_path(&mut self, _: &BezPath) {}
    fn fill_rect(&mut self, _: &Rect) {}
    fn push_clip_layer(&mut self, _: &BezPath) {
        self.layer();
    }
    fn push_clip_path(&mut self, _: &BezPath) {}
    fn pop_clip(&mut self) {}
    fn push_opacity_layer(&mut self, _: f32) {
        self.layer();
    }
    fn push_filter_layer(&mut self, filter: Filter, _: f64) {
        self.layer();
        if !supported_filter(&filter) {
            self.refuse(GpuUnsupported::FilterGraph);
            return;
        }
        let t = self.transform;
        let far = reach(filter.filter_expansion(&t)).max(reach(filter.source_expansion(&t)));
        // NaN (a broken transform) is refused too.
        if far.is_nan() || far > MAX_FILTER_REACH {
            self.refuse(GpuUnsupported::FilterReach);
        }
    }
    fn pop_layer(&mut self) {}
}

impl Canvas for Scene {
    fn set_transform(&mut self, t: Affine) {
        Scene::set_transform(self, t);
    }
    fn set_paint(&mut self, paint: PaintType) {
        Scene::set_paint(self, paint);
    }
    fn set_paint_transform(&mut self, t: Affine) {
        Scene::set_paint_transform(self, t);
    }
    fn set_fill_rule(&mut self, rule: Fill) {
        Scene::set_fill_rule(self, rule);
    }
    fn fill_path(&mut self, path: &BezPath) {
        Scene::fill_path(self, path);
    }
    fn fill_rect(&mut self, rect: &Rect) {
        Scene::fill_rect(self, rect);
    }
    fn push_clip_layer(&mut self, path: &BezPath) {
        Scene::push_clip_layer(self, path);
    }
    fn push_clip_path(&mut self, path: &BezPath) {
        Scene::push_clip_path(self, path);
    }
    fn pop_clip(&mut self) {
        Scene::pop_clip(self);
    }
    fn push_opacity_layer(&mut self, opacity: f32) {
        Scene::push_opacity_layer(self, opacity);
    }
    fn push_filter_layer(&mut self, filter: Filter, _reach: f64) {
        Scene::push_filter_layer(self, filter);
    }
    fn pop_layer(&mut self) {
        Scene::pop_layer(self);
    }
}

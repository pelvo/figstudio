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
use vello_cpu::filter_effects::Filter;
use vello_cpu::kurbo::{Affine, BezPath, Rect};
use vello_cpu::peniko::{BlendMode, Fill};
use vello_gpu::Scene;

use crate::Viewport;
use crate::gpu_scan::Scan;
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
    /// A filter whose layer, grown by the filters around it, would not fit the GPU's
    /// intermediate textures (`vello_gpu` fails the render with `TooLarge`).
    FilterReach,
    /// More than [`MAX_LAYERS`] layers in the tile (`vello_gpu`'s scheduler gives up, with a
    /// panic, on extreme scenes; its filter data texture has a fixed size).
    Layers,
    /// The scene would keep more memory than the caller allows
    /// ([`Recording::gpu_scene_within`]).
    TooBig,
    /// A filter layer, refused by [`Recording::gpu_scene_within`]: `vello_gpu` grows its strip
    /// viewport by each filter's padding, which the memory estimate does not bound.
    Filtered,
}

impl fmt::Display for GpuUnsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            GpuUnsupported::Image => "image paint",
            GpuUnsupported::SweepGradient => "angular gradient",
            GpuUnsupported::FilterGraph => "filter not supported on the GPU",
            GpuUnsupported::FilterReach => "filter reaches too far for the GPU",
            GpuUnsupported::Layers => "too many layers for the GPU",
            GpuUnsupported::TooBig => "too big a scene for the GPU",
            GpuUnsupported::Filtered => "filter in a size-bounded GPU scene",
        })
    }
}

impl std::error::Error for GpuUnsupported {}

/// `vello_gpu`'s default largest intermediate texture side (`LayersConfig::max_texture_size`),
/// before it is lowered to the device's texture limit: pass the smaller of the two to
/// [`Recording::gpu_scene`].
pub const GPU_LAYER_TEXTURE: u16 = 4096;
/// Most layers (clip, opacity and filter) one GPU tile may push.
pub const MAX_LAYERS: usize = 10_000;

impl Recording {
    /// Builds the `vello_gpu` scene of `viewport`, as [`Recording::render`] would draw it on the
    /// CPU (with its background). Returns the scene and how many fills were replayed,
    /// `Some(Err(_))` when the tile uses something `vello_gpu` cannot draw (draw it with
    /// [`Recording::render`]), or `None` when [`Recording::render`] would.
    ///
    /// `max_texture`: the renderer's largest intermediate texture side, the smaller of
    /// [`GPU_LAYER_TEXTURE`] and the device's `max_texture_dimension_2d`. Rendering the scene
    /// can still fail (`vello_gpu::RenderError`); draw the tile on the CPU then.
    pub fn gpu_scene(
        &self,
        viewport: &Viewport,
        max_texture: u16,
    ) -> Option<Result<(Scene, usize), GpuUnsupported>> {
        let (window, shift) = self.place(viewport)?;
        let (w, h) = (viewport.width.max(1), viewport.height.max(1));
        let mut scan = Scan::new(w, h, max_texture, false);
        self.rec.replay(&mut scan, window, shift);
        if let Err(why) = scan.verdict() {
            return Some(Err(why));
        }
        Some(Ok(self.scene(viewport, window, shift)))
    }

    /// Like [`Recording::gpu_scene`], for a caller that bounds scene memory: refuses any filter
    /// layer ([`GpuUnsupported::Filtered`]; `vello_gpu` grows its strip viewport by each
    /// filter's padding, which the estimate does not cover), then offers `admit` a
    /// conservative estimate of the bytes the scene will keep (`gpu_bytes.rs`) before building
    /// it. `admit` returns what to keep with the scene (a reservation of those bytes), or
    /// `None` to refuse it ([`GpuUnsupported::TooBig`]).
    pub fn gpu_scene_within<R>(
        &self,
        viewport: &Viewport,
        max_texture: u16,
        admit: impl FnOnce(usize) -> Option<R>,
    ) -> Option<Result<(Scene, usize, R), GpuUnsupported>> {
        let (window, shift) = self.place(viewport)?;
        let (w, h) = (viewport.width.max(1), viewport.height.max(1));
        let mut scan = Scan::new(w, h, max_texture, true);
        self.rec.replay(&mut scan, window, shift);
        let admitted = match scan.verdict() {
            Ok(bytes) => admit(bytes),
            Err(why) => return Some(Err(why)),
        };
        let Some(kept) = admitted else {
            return Some(Err(GpuUnsupported::TooBig));
        };
        let (scene, fills) = self.scene(viewport, window, shift);
        Some(Ok((scene, fills, kept)))
    }

    /// Replays the tile into a new scene (after the scan accepted it).
    fn scene(&self, viewport: &Viewport, window: Rect, shift: Affine) -> (Scene, usize) {
        let (w, h) = (viewport.width.max(1), viewport.height.max(1));
        let mut scene = Scene::new(w, h);
        if let Some(bg) = viewport.background {
            scene.set_paint(bg);
            scene.fill_rect(&Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
        }
        let fills = self.rec.replay(&mut scene, window, shift);
        (scene, fills)
    }
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
    fn set_blend_mode(&mut self, mode: BlendMode) {
        Scene::set_blend_mode(self, mode);
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

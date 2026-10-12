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
use vello_cpu::kurbo::{Affine, BezPath, Rect, Shape};
use vello_cpu::peniko::{Fill, GradientKind};
use vello_gpu::Scene;

use crate::Viewport;
use crate::gpu_bytes::SceneBytes;
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
        })
    }
}

impl std::error::Error for GpuUnsupported {}

/// `vello_gpu`'s default largest intermediate texture side (`LayersConfig::max_texture_size`),
/// before it is lowered to the device's texture limit: pass the smaller of the two to
/// [`Recording::gpu_scene`].
pub const GPU_LAYER_TEXTURE: u16 = 4096;
/// `vello_gpu`'s `FILTER_ATLAS_PADDING` (`MAX_KERNEL_SIZE / 2`), added on each side of the
/// largest filter layer when sizing intermediate textures.
const FILTER_ATLAS_PADDING: f64 = 6.0;
/// `vello_common`'s tile side: filter paddings and layer bounds are snapped up to it.
const TILE: f64 = 4.0;
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
        self.build(viewport, max_texture, None)
            .map(|r| r.map(|(scene, fills, _)| (scene, fills)))
    }

    /// Like [`Recording::gpu_scene`], but refuses ([`GpuUnsupported::TooBig`]) a scene that
    /// would keep more than `max_bytes` (a conservative estimate made before building it, see
    /// `gpu_bytes.rs`), and returns the estimate with the scene and its fill count.
    pub fn gpu_scene_within(
        &self,
        viewport: &Viewport,
        max_texture: u16,
        max_bytes: usize,
    ) -> Option<Result<(Scene, usize, usize), GpuUnsupported>> {
        self.build(viewport, max_texture, Some(max_bytes))
    }

    fn build(
        &self,
        viewport: &Viewport,
        max_texture: u16,
        max_bytes: Option<usize>,
    ) -> Option<Result<(Scene, usize, usize), GpuUnsupported>> {
        let (window, shift) = self.place(viewport)?;
        let (w, h) = (viewport.width.max(1), viewport.height.max(1));
        let mut scan = Scan {
            transform: Affine::IDENTITY,
            layers: 0,
            viewports: vec![(f64::from(w), f64::from(h))],
            max: f64::from(max_texture),
            unsupported: None,
            bytes: max_bytes.map(|_| SceneBytes::default()),
            view: Rect::new(0.0, 0.0, f64::from(w), f64::from(h)),
        };
        self.rec.replay(&mut scan, window, shift);
        if let Some(why) = scan.unsupported {
            return Some(Err(why));
        }
        let bytes = scan.bytes.as_ref().map_or(0, SceneBytes::total);
        if max_bytes.is_some_and(|max| bytes > max) {
            return Some(Err(GpuUnsupported::TooBig));
        }
        let mut scene = Scene::new(w, h);
        if let Some(bg) = viewport.background {
            scene.set_paint(bg);
            scene.fill_rect(&Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
        }
        let fills = self.rec.replay(&mut scene, window, shift);
        Some(Ok((scene, fills, bytes)))
    }
}

/// Checks the commands a tile replays; keeps the first reason it cannot go to the GPU.
struct Scan {
    transform: Affine,
    layers: usize,
    /// Strip viewport size per open layer (the scene's first). `vello_gpu` grows it by each
    /// filter's source padding (`ViewportState::push_root_viewport`), so nested filters add up.
    viewports: Vec<(f64, f64)>,
    /// Largest intermediate texture side.
    max: f64,
    unsupported: Option<GpuUnsupported>,
    /// The scene's memory estimate, when asked for.
    bytes: Option<SceneBytes>,
    /// The scene's viewport, in pixels.
    view: Rect,
}

impl Scan {
    fn refuse(&mut self, why: GpuUnsupported) {
        self.unsupported.get_or_insert(why);
    }

    /// Opens a layer with strip viewport `size`.
    fn layer(&mut self, size: (f64, f64)) {
        self.layers += 1;
        if self.layers > MAX_LAYERS {
            self.refuse(GpuUnsupported::Layers);
        }
        self.viewports.push(size);
    }

    fn viewport(&self) -> (f64, f64) {
        self.viewports.last().copied().unwrap_or_default()
    }

    /// Refuses `filter` unless its intermediate texture fits, as `vello_gpu` 0.3 sizes it:
    /// `FilterData::new` snaps both paddings up to tiles; the layer's pixmap is at most the grown
    /// viewport plus the filter padding (`FilterLayerPlacement::new`), plus the atlas padding on
    /// each side (`LayersConfig::required_intermediate_texture_size`). Returns the grown viewport.
    fn fit_filter(&mut self, filter: &Filter) -> (f64, f64) {
        let snap = |v: f64| (v.max(0.0) / TILE).ceil() * TILE;
        let pad = |r: Rect| (snap(-r.x0) + snap(r.x1), snap(-r.y0) + snap(r.y1));
        let (sw, sh) = pad(filter.source_expansion(&self.transform));
        let (fw, fh) = pad(filter.filter_expansion(&self.transform));
        let (w, h) = self.viewport();
        let grown = (w + sw, h + sh);
        let atlas = 2.0 * FILTER_ATLAS_PADDING + TILE;
        let (lw, lh) = (grown.0 + fw + atlas, grown.1 + fh + atlas);
        // NaN (a broken transform) is refused too.
        if !(lw <= self.max && lh <= self.max) {
            self.refuse(GpuUnsupported::FilterReach);
        }
        grown
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

impl Canvas for Scan {
    fn set_transform(&mut self, t: Affine) {
        self.transform = t;
    }
    fn set_paint(&mut self, paint: PaintType) {
        if let Some(b) = &mut self.bytes {
            b.set_paint(&paint);
        }
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
    fn fill_path(&mut self, path: &BezPath) {
        if let Some(b) = &mut self.bytes {
            b.fill(path, self.transform, self.view);
        }
    }
    fn fill_rect(&mut self, rect: &Rect) {
        if let Some(b) = &mut self.bytes {
            b.fill(&rect.to_path(0.1), self.transform, self.view);
        }
    }
    fn push_clip_layer(&mut self, path: &BezPath) {
        if let Some(b) = &mut self.bytes {
            b.layer(Some(path), self.transform, self.view);
        }
        self.layer(self.viewport());
    }
    fn push_clip_path(&mut self, path: &BezPath) {
        if let Some(b) = &mut self.bytes {
            b.layer(Some(path), self.transform, self.view);
        }
    }
    fn pop_clip(&mut self) {}
    fn push_opacity_layer(&mut self, _: f32) {
        if let Some(b) = &mut self.bytes {
            b.layer(None, self.transform, self.view);
        }
        self.layer(self.viewport());
    }
    fn push_filter_layer(&mut self, filter: Filter, _: f64) {
        if let Some(b) = &mut self.bytes {
            b.layer(None, self.transform, self.view);
        }
        let size = if supported_filter(&filter) {
            self.fit_filter(&filter)
        } else {
            self.refuse(GpuUnsupported::FilterGraph);
            self.viewport()
        };
        self.layer(size);
    }
    fn pop_layer(&mut self) {
        // The scene's own viewport stays (an unbalanced pop is not ours to fix here).
        if self.viewports.len() > 1 {
            self.viewports.pop();
        }
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

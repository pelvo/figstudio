//! The support scan: replays a tile's commands without drawing, to refuse what `vello_gpu`
//! 0.3 cannot draw before a scene is built, and (when asked) to estimate the scene's memory.

use vello_cpu::PaintType;
use vello_cpu::filter_effects::{Filter, FilterPrimitive};
use vello_cpu::kurbo::{Affine, BezPath, Rect, Shape};
use vello_cpu::peniko::{BlendMode, Fill, GradientKind};

use crate::gpu::{GpuUnsupported, MAX_LAYERS};
use crate::gpu_bytes::SceneBytes;
use crate::record::Canvas;

/// `vello_gpu`'s `FILTER_ATLAS_PADDING` (`MAX_KERNEL_SIZE / 2`), added on each side of the
/// largest filter layer when sizing intermediate textures.
const FILTER_ATLAS_PADDING: f64 = 6.0;
/// `vello_common`'s tile side: filter paddings and layer bounds are snapped up to it.
const TILE: f64 = 4.0;

/// Checks the commands a tile replays; keeps the first reason it cannot go to the GPU.
pub(crate) struct Scan {
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
    /// Refuse every filter layer ([`GpuUnsupported::Filtered`]): the estimate does not cover
    /// them (`vello_gpu` grows the strip viewport by each filter's padding).
    no_filters: bool,
}

impl Scan {
    /// A scan of a `w` × `h` px scene for a renderer whose largest intermediate texture side is
    /// `max_texture`; `estimate`: also estimate its memory, and refuse filters.
    pub(crate) fn new(w: u16, h: u16, max_texture: u16, estimate: bool) -> Self {
        Scan {
            transform: Affine::IDENTITY,
            layers: 0,
            viewports: vec![(f64::from(w), f64::from(h))],
            max: f64::from(max_texture),
            unsupported: None,
            bytes: estimate.then(SceneBytes::default),
            view: Rect::new(0.0, 0.0, f64::from(w), f64::from(h)),
            no_filters: estimate,
        }
    }

    /// The first reason the tile cannot go to the GPU, else the memory estimate (0 when not
    /// asked for).
    pub(crate) fn verdict(&self) -> Result<usize, GpuUnsupported> {
        match self.unsupported {
            Some(why) => Err(why),
            None => Ok(self.bytes.as_ref().map_or(0, SceneBytes::total)),
        }
    }

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
    fn set_blend_mode(&mut self, _: BlendMode) {}
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
        if self.no_filters {
            self.refuse(GpuUnsupported::Filtered);
        }
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

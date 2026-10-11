//! Where drawing goes: straight into a Vello render context, or (feature `parallel`) into a
//! `Recorder` whose commands are replayed into several contexts, one per strip of a parallel
//! render, so the document is walked once however many strips there are.

use vello_cpu::PaintType;
use vello_cpu::RenderContext;
use vello_cpu::filter_effects::Filter;
use vello_cpu::kurbo::Rect;
use vello_cpu::kurbo::{Affine, BezPath};
use vello_cpu::peniko::{BlendMode, Compose, Fill, Mix};

/// The drawing calls the renderer makes (a subset of `RenderContext`).
pub(crate) trait Canvas {
    fn set_transform(&mut self, t: Affine);
    fn set_paint(&mut self, paint: PaintType);
    fn set_paint_transform(&mut self, t: Affine);
    fn set_fill_rule(&mut self, rule: Fill);
    fn fill_path(&mut self, path: &BezPath);
    fn push_clip_layer(&mut self, path: &BezPath);
    /// A clip without a layer (cheaper); undone by [`Canvas::pop_clip`], nested like layers.
    fn push_clip_path(&mut self, path: &BezPath);
    fn pop_clip(&mut self);
    fn push_opacity_layer(&mut self, opacity: f32);
    /// `reach`: how far (in local units) the filter can move or spread what is drawn in it.
    fn push_filter_layer(&mut self, filter: Filter, reach: f64);
    fn pop_layer(&mut self);
    /// Erases everything drawn so far in the innermost layer (a discarded partial redraw).
    fn clear_layer(&mut self, area: Rect);
}

impl Canvas for RenderContext {
    fn set_transform(&mut self, t: Affine) {
        RenderContext::set_transform(self, t);
    }
    fn set_paint(&mut self, paint: PaintType) {
        RenderContext::set_paint(self, paint);
    }
    fn set_paint_transform(&mut self, t: Affine) {
        RenderContext::set_paint_transform(self, t);
    }
    fn set_fill_rule(&mut self, rule: Fill) {
        RenderContext::set_fill_rule(self, rule);
    }
    fn fill_path(&mut self, path: &BezPath) {
        RenderContext::fill_path(self, path);
    }
    fn push_clip_layer(&mut self, path: &BezPath) {
        RenderContext::push_clip_layer(self, path);
    }
    fn push_clip_path(&mut self, path: &BezPath) {
        RenderContext::push_clip_path(self, path);
    }
    fn pop_clip(&mut self) {
        RenderContext::pop_clip(self);
    }
    fn push_opacity_layer(&mut self, opacity: f32) {
        RenderContext::push_opacity_layer(self, opacity);
    }
    fn push_filter_layer(&mut self, filter: Filter, _reach: f64) {
        RenderContext::push_filter_layer(self, filter);
    }
    fn pop_layer(&mut self) {
        RenderContext::pop_layer(self);
    }
    fn clear_layer(&mut self, area: Rect) {
        RenderContext::set_transform(self, Affine::IDENTITY);
        RenderContext::set_blend_mode(self, BlendMode::new(Mix::Normal, Compose::Clear));
        RenderContext::fill_rect(self, &area);
        RenderContext::set_blend_mode(self, BlendMode::new(Mix::Normal, Compose::SrcOver));
    }
}

#[cfg(feature = "parallel")]
use crate::paint::linear_scale;

#[cfg(feature = "parallel")]
enum Layer {
    Clip(BezPath),
    /// `push_clip_path`: popped with `pop_clip`, not `pop_layer`.
    ClipPath(BezPath),
    Opacity(f32),
    Filter(Filter),
}

#[cfg(feature = "parallel")]
enum Cmd {
    Transform(Affine),
    Paint(PaintType),
    PaintTransform(Affine),
    FillRule(Fill),
    Fill(BezPath, Rect),
    FillRect(Rect, Rect),
    /// A layer, the output area its contents can change, and the index of its `Pop`.
    Push(Layer, Rect, usize),
    Pop,
    PopClip,
    /// Erase the innermost layer within the given output area.
    Clear(Rect),
}

/// Commands recorded in output pixel coordinates, with the output area each fill and layer
/// can change, so a replay can skip what lies outside its strip.
#[cfg(feature = "parallel")]
pub(crate) struct Recorder {
    cmds: Vec<Cmd>,
    transform: Affine,
    /// Open layers: index of the `Push`, its contents' area so far, and the filter reach in
    /// output pixels that its contents are spread by (summed over enclosing filters).
    open: Vec<(usize, Rect, f64)>,
}

/// An empty area (`union` with it gives the other rectangle).
#[cfg(feature = "parallel")]
const NOTHING: Rect = Rect::new(
    f64::INFINITY,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::NEG_INFINITY,
);

#[cfg(feature = "parallel")]
fn union(a: Rect, b: Rect) -> Rect {
    Rect::new(
        a.x0.min(b.x0),
        a.y0.min(b.y0),
        a.x1.max(b.x1),
        a.y1.max(b.y1),
    )
}

#[cfg(feature = "parallel")]
impl Recorder {
    pub(crate) fn new() -> Self {
        Recorder {
            cmds: Vec::new(),
            transform: Affine::IDENTITY,
            open: Vec::new(),
        }
    }

    fn reach(&self) -> f64 {
        self.open.last().map_or(0.0, |o| o.2)
    }

    /// Notes that something covering `local` (in the current transform's space) was drawn,
    /// and returns the output area it can change.
    fn touched(&mut self, local: Rect) -> Rect {
        // One extra pixel for anti-aliasing at the edges.
        let r = self.reach() + 1.0;
        let area = self.transform.transform_rect_bbox(local).inflate(r, r);
        if let Some(top) = self.open.last_mut() {
            top.1 = union(top.1, area);
        }
        area
    }

    pub(crate) fn fill_rect(&mut self, rect: &Rect) {
        let area = self.touched(*rect);
        self.cmds.push(Cmd::FillRect(*rect, area));
    }

    fn push(&mut self, layer: Layer, reach: f64) {
        self.open.push((self.cmds.len(), NOTHING, reach));
        self.cmds.push(Cmd::Push(layer, NOTHING, 0));
    }

    /// Replays every command that can change output rows `y0..y1` into `rc`, shifted so row
    /// `y0` is the context's first row.
    pub(crate) fn replay(&self, rc: &mut RenderContext, y0: f64, y1: f64) {
        let shift = Affine::translate((0.0, -y0));
        let hits = |r: &Rect| r.y1 > y0 && r.y0 < y1;
        let mut i = 0;
        while i < self.cmds.len() {
            match &self.cmds[i] {
                Cmd::Transform(t) => rc.set_transform(shift * *t),
                Cmd::Paint(p) => rc.set_paint(p.clone()),
                Cmd::PaintTransform(t) => rc.set_paint_transform(*t),
                Cmd::FillRule(f) => rc.set_fill_rule(*f),
                Cmd::Fill(path, area) => {
                    if hits(area) {
                        rc.fill_path(path);
                    }
                }
                Cmd::FillRect(rect, area) => {
                    if hits(area) {
                        rc.fill_rect(rect);
                    }
                }
                Cmd::Push(layer, area, end) => {
                    if !hits(area) {
                        // Nothing inside can reach these rows. State set inside is set again
                        // before it is used, so skipping the whole layer is safe.
                        i = end + 1;
                        continue;
                    }
                    match layer {
                        Layer::Clip(p) => rc.push_clip_layer(p),
                        Layer::ClipPath(p) => rc.push_clip_path(p),
                        Layer::Opacity(o) => rc.push_opacity_layer(*o),
                        Layer::Filter(f) => rc.push_filter_layer(f.clone()),
                    }
                }
                Cmd::Pop => rc.pop_layer(),
                Cmd::PopClip => rc.pop_clip(),
                Cmd::Clear(area) => {
                    if hits(area) {
                        Canvas::clear_layer(
                            rc,
                            Rect::new(area.x0, area.y0 - y0, area.x1, area.y1 - y0),
                        );
                    }
                }
            }
            i += 1;
        }
    }
}

#[cfg(feature = "parallel")]
impl Canvas for Recorder {
    fn set_transform(&mut self, t: Affine) {
        self.transform = t;
        self.cmds.push(Cmd::Transform(t));
    }
    fn set_paint(&mut self, paint: PaintType) {
        self.cmds.push(Cmd::Paint(paint));
    }
    fn set_paint_transform(&mut self, t: Affine) {
        self.cmds.push(Cmd::PaintTransform(t));
    }
    fn set_fill_rule(&mut self, rule: Fill) {
        self.cmds.push(Cmd::FillRule(rule));
    }
    fn fill_path(&mut self, path: &BezPath) {
        let area = self.touched(path.control_box());
        self.cmds.push(Cmd::Fill(path.clone(), area));
    }
    fn push_clip_layer(&mut self, path: &BezPath) {
        let r = self.reach();
        self.push(Layer::Clip(path.clone()), r);
    }
    fn push_clip_path(&mut self, path: &BezPath) {
        let r = self.reach();
        self.push(Layer::ClipPath(path.clone()), r);
    }
    fn pop_clip(&mut self) {
        self.close(Cmd::PopClip);
    }
    fn push_opacity_layer(&mut self, opacity: f32) {
        let r = self.reach();
        self.push(Layer::Opacity(opacity), r);
    }
    fn push_filter_layer(&mut self, filter: Filter, reach: f64) {
        let r = self.reach() + reach.abs() * linear_scale(self.transform);
        self.push(Layer::Filter(filter), r);
    }
    fn pop_layer(&mut self) {
        self.close(Cmd::Pop);
    }
    fn clear_layer(&mut self, area: Rect) {
        // Drawing in the layer: its output area covers the erase too, or a strip that only the
        // erase reaches would skip the whole layer and keep the discarded redraw's neighbours.
        // `area` is in output pixels: record it under the identity transform.
        self.transform = Affine::IDENTITY;
        let area = self.touched(area);
        self.cmds.push(Cmd::Clear(area));
    }
}

#[cfg(feature = "parallel")]
impl Recorder {
    /// Ends the innermost layer or clip with `pop`.
    fn close(&mut self, pop: Cmd) {
        let Some((at, area, _)) = self.open.pop() else {
            return;
        };
        let end = self.cmds.len();
        self.cmds.push(pop);
        if let Cmd::Push(_, a, e) = &mut self.cmds[at] {
            *a = area;
            *e = end;
        }
        if let Some(top) = self.open.last_mut() {
            top.1 = union(top.1, area);
        }
    }
}

//! Where drawing goes: straight into a Vello render context, or (feature `parallel`) into a
//! `Recorder` whose commands are replayed into several contexts, one per strip of a parallel
//! render, so the document is walked once however many strips there are.

use vello_cpu::PaintType;
use vello_cpu::RenderContext;
use vello_cpu::filter_effects::Filter;
use vello_cpu::kurbo::{Affine, BezPath, Rect};
use vello_cpu::peniko::Fill;

/// The drawing calls the renderer makes (a subset of `RenderContext`).
pub(crate) trait Canvas {
    fn set_transform(&mut self, t: Affine);
    fn set_paint(&mut self, paint: PaintType);
    fn set_paint_transform(&mut self, t: Affine);
    fn set_fill_rule(&mut self, rule: Fill);
    fn fill_path(&mut self, path: &BezPath);
    #[cfg_attr(
        not(feature = "parallel"),
        expect(dead_code, reason = "only replays call it through the trait")
    )]
    fn fill_rect(&mut self, rect: &Rect);
    fn push_clip_layer(&mut self, path: &BezPath);
    /// A clip without a layer (cheaper); undone by [`Canvas::pop_clip`], nested like layers.
    fn push_clip_path(&mut self, path: &BezPath);
    fn pop_clip(&mut self);
    fn push_opacity_layer(&mut self, opacity: f32);
    /// `reach`: how far (in local units) the filter can move or spread what is drawn in it.
    fn push_filter_layer(&mut self, filter: Filter, reach: f64);
    fn pop_layer(&mut self);
    /// True once a recording is over its size limit: the walk stops (its output is discarded).
    fn full(&self) -> bool {
        false
    }
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
    fn fill_rect(&mut self, rect: &Rect) {
        RenderContext::fill_rect(self, rect);
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
}

#[cfg(feature = "parallel")]
fn path_bytes(p: &BezPath) -> usize {
    // A clone's vector has exactly the elements' length as capacity.
    size_of_val(p.elements())
}

#[cfg(feature = "parallel")]
impl Cmd {
    /// Heap bytes this command owns (conservative). Image pixels are shared with the image
    /// cache and not counted.
    fn heap_bytes(&self) -> usize {
        match self {
            Cmd::Fill(p, _) | Cmd::Push(Layer::Clip(p) | Layer::ClipPath(p), _, _) => path_bytes(p),
            Cmd::Paint(PaintType::Gradient(g)) => {
                g.stops.capacity() * size_of::<vello_cpu::peniko::ColorStop>()
            }
            // The graph is shared (`Arc`), but each push makes its own: count its primitives.
            Cmd::Push(Layer::Filter(f), _, _) => {
                size_of_val(&*f.graph)
                    + f.graph.primitives.capacity()
                        * size_of::<vello_cpu::filter_effects::FilterPrimitive>()
            }
            _ => 0,
        }
    }
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
    /// Estimated bytes held by `cmds`, and the most allowed; over it, everything is dropped.
    bytes: usize,
    limit: usize,
    over: bool,
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
        Self::with_limit(usize::MAX)
    }

    /// A recorder that gives up (see [`Recorder::over`]) past about `limit` bytes.
    pub(crate) fn with_limit(limit: usize) -> Self {
        Recorder {
            cmds: Vec::new(),
            transform: Affine::IDENTITY,
            open: Vec::new(),
            bytes: 0,
            limit,
            over: false,
        }
    }

    /// The recording passed its limit and holds nothing.
    pub(crate) fn over(&self) -> bool {
        self.over
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// Appends `cmd` unless that passes the limit (checked before it is kept).
    fn add(&mut self, cmd: Cmd) {
        if self.over {
            return;
        }
        // The command, its vector slot once grown (up to twice its size), and what it owns.
        self.bytes = self
            .bytes
            .saturating_add(2 * size_of::<Cmd>())
            .saturating_add(cmd.heap_bytes());
        if self.bytes > self.limit {
            self.over = true;
            self.cmds = Vec::new();
            self.open = Vec::new();
            return;
        }
        self.cmds.push(cmd);
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

    fn push(&mut self, layer: Layer, reach: f64) {
        let at = self.cmds.len();
        self.add(Cmd::Push(layer, NOTHING, 0));
        if !self.over {
            self.open.push((at, NOTHING, reach));
        }
    }

    /// Replays into `rc` (a CPU context, the GPU scene or the GPU support scan of `gpu.rs`)
    /// every command that can change the recorded pixels `window`, each transform preceded by
    /// `shift` (recorded pixels to `rc` pixels). Returns how many fills were replayed.
    pub(crate) fn replay(&self, rc: &mut impl Canvas, window: Rect, shift: Affine) -> usize {
        let hits =
            |r: &Rect| r.x1 > window.x0 && r.x0 < window.x1 && r.y1 > window.y0 && r.y0 < window.y1;
        let mut fills = 0;
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
                        fills += 1;
                    }
                }
                Cmd::FillRect(rect, area) => {
                    if hits(area) {
                        rc.fill_rect(rect);
                        fills += 1;
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
                        Layer::Filter(f) => rc.push_filter_layer(f.clone(), 0.0),
                    }
                }
                Cmd::Pop => rc.pop_layer(),
                Cmd::PopClip => rc.pop_clip(),
            }
            i += 1;
        }
        fills
    }
}

#[cfg(feature = "parallel")]
impl Canvas for Recorder {
    fn set_transform(&mut self, t: Affine) {
        self.transform = t;
        self.add(Cmd::Transform(t));
    }
    fn set_paint(&mut self, paint: PaintType) {
        self.add(Cmd::Paint(paint));
    }
    fn set_paint_transform(&mut self, t: Affine) {
        self.add(Cmd::PaintTransform(t));
    }
    fn set_fill_rule(&mut self, rule: Fill) {
        self.add(Cmd::FillRule(rule));
    }
    fn fill_path(&mut self, path: &BezPath) {
        let area = self.touched(path.control_box());
        self.add(Cmd::Fill(path.clone(), area));
    }
    fn fill_rect(&mut self, rect: &Rect) {
        let area = self.touched(*rect);
        self.add(Cmd::FillRect(*rect, area));
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
    fn full(&self) -> bool {
        self.over
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
        self.add(pop);
        if self.over {
            return;
        }
        if let Cmd::Push(_, a, e) = &mut self.cmds[at] {
            *a = area;
            *e = end;
        }
        if let Some(top) = self.open.last_mut() {
            top.1 = union(top.1, area);
        }
    }
}
